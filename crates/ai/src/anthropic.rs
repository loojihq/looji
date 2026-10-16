//! The Anthropic Messages API, streamed. Vendor types stay in this file. Callers see `ChatProvider`, `ChatEvent`,
//! and `AiError`.
//!
//! The parsing and the status mapping are pure functions, so the fixtures under `tests/fixtures` test them
//! without a network. `AnthropicProvider` only adds the request and the reading loop.

use std::time::Duration;

use futures_util::{StreamExt, future::BoxFuture};
use serde::Serialize;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::{
    error::AiError,
    key::ApiKey,
    provider::{ChatProvider, EventStream, Producer, open_stream},
    sse::{SseDecoder, SseEvent},
    types::{ChatEvent, ChatMessage, ChatRequest, ChatRole, Usage},
};

pub const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";
const API_VERSION: &str = "2023-06-01";

#[derive(Serialize)]
struct WireMessage<'a> {
    role: &'static str,
    content: &'a str,
}

#[derive(Serialize)]
struct WireRequest<'a> {
    model: &'a str,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<&'a str>,
    messages: Vec<WireMessage<'a>>,
    stream: bool,
}

/// The JSON body for a request. The key isn't part of the request, so it can't end up here.
pub fn request_body(request: &ChatRequest) -> Value {
    let wire = WireRequest {
        model: &request.model,
        max_tokens: request.max_tokens,
        system: request.system.as_deref(),
        messages: request.messages.iter().map(wire_message).collect(),
        stream: true,
    };
    serde_json::to_value(wire).expect("a request body always serializes")
}

fn wire_message(message: &ChatMessage) -> WireMessage<'_> {
    WireMessage {
        role: match message.role {
            ChatRole::User => "user",
            ChatRole::Assistant => "assistant",
        },
        content: &message.content,
    }
}

/// What the stream has reported so far. The counts come in two events, so they're kept until the end.
#[derive(Debug, Default, Clone, Copy)]
pub struct StreamUsage {
    input_tokens: Option<u32>,
    output_tokens: Option<u32>,
}

impl StreamUsage {
    fn report(self) -> Option<Usage> {
        (self.input_tokens.is_some() || self.output_tokens.is_some()).then(|| Usage {
            input_tokens: self.input_tokens.unwrap_or(0),
            output_tokens: self.output_tokens.unwrap_or(0),
        })
    }
}

fn token_count(value: &Value) -> Option<u32> {
    value.as_u64().and_then(|count| u32::try_from(count).ok())
}

/// Reads one server-sent event into a `ChatEvent`. Returns `None` for events that carry nothing the caller needs,
/// such as pings, block boundaries, and tool input. An in-stream error becomes an `AiError`.
pub fn to_chat_event(
    event: &SseEvent,
    usage: &mut StreamUsage,
) -> Result<Option<ChatEvent>, AiError> {
    let value: Value = serde_json::from_str(&event.data).map_err(|_| {
        // The data isn't echoed, since it may hold text from the conversation.
        AiError::Protocol("an event's data was not JSON".to_string())
    })?;
    let kind = value
        .get("type")
        .and_then(Value::as_str)
        .or(event.event.as_deref())
        .unwrap_or("");
    match kind {
        "message_start" => {
            usage.input_tokens = token_count(&value["message"]["usage"]["input_tokens"]);
            Ok(None)
        }
        "message_delta" => {
            if let Some(output) = token_count(&value["usage"]["output_tokens"]) {
                usage.output_tokens = Some(output);
            }
            Ok(None)
        }
        "content_block_delta" => match value["delta"]["type"].as_str() {
            Some("text_delta") => Ok(value["delta"]["text"]
                .as_str()
                .map(|text| ChatEvent::Token(text.to_string()))),
            _ => Ok(None),
        },
        "message_stop" => Ok(Some(ChatEvent::Finished {
            usage: usage.report(),
        })),
        "error" => Err(error_for_kind(
            value["error"]["type"].as_str().unwrap_or(""),
        )),
        _ => Ok(None),
    }
}

/// The error for an in-stream error event, by the type the provider gave it.
pub fn error_for_kind(kind: &str) -> AiError {
    match kind {
        "authentication_error" | "permission_error" => AiError::AuthFailed,
        "rate_limit_error" => AiError::RateLimited { retry_after: None },
        "overloaded_error" => AiError::Unavailable { status: 529 },
        "api_error" => AiError::Unavailable { status: 500 },
        other => AiError::Protocol(format!("the provider reported an error of type {other}")),
    }
}

/// The error for a response that isn't a success. `retry_after` is the `retry-after` header's value.
pub fn error_for_status(status: u16, retry_after: Option<&str>) -> AiError {
    match status {
        401 | 403 => AiError::AuthFailed,
        429 => AiError::RateLimited {
            retry_after: retry_after.and_then(parse_retry_after),
        },
        500..=599 => AiError::Unavailable { status },
        _ => AiError::Rejected { status },
    }
}

/// A `retry-after` value in seconds. Dates are not read, since the provider sends seconds.
pub fn parse_retry_after(value: &str) -> Option<Duration> {
    value.trim().parse::<u64>().ok().map(Duration::from_secs)
}

fn network_error(err: reqwest::Error) -> AiError {
    // `without_url` keeps the address out of the message. The key is in a header and never in the URL, but this
    // keeps the message to what failed.
    AiError::Network(err.without_url().to_string())
}

/// Anthropic's Messages API, over HTTPS.
pub struct AnthropicProvider {
    key: ApiKey,
    client: reqwest::Client,
    base_url: String,
}

impl AnthropicProvider {
    pub fn new(key: ApiKey) -> Self {
        Self {
            key,
            client: reqwest::Client::new(),
            base_url: DEFAULT_BASE_URL.to_string(),
        }
    }

    /// Points the provider at another server. The app uses the default. Tests use this.
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }
}

impl ChatProvider for AnthropicProvider {
    fn stream_chat(
        &self,
        request: ChatRequest,
        cancel: CancellationToken,
    ) -> BoxFuture<'_, Result<EventStream, AiError>> {
        Box::pin(async move {
            let body = request_body(&request);
            let send = self
                .client
                .post(format!("{}/v1/messages", self.base_url))
                .header("x-api-key", self.key.expose())
                .header("anthropic-version", API_VERSION)
                .json(&body)
                .send();
            let response = tokio::select! {
                biased;
                _ = cancel.cancelled() => return Err(AiError::Cancelled),
                sent = send => sent.map_err(network_error)?,
            };
            let status = response.status();
            if !status.is_success() {
                let retry_after = response
                    .headers()
                    .get("retry-after")
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_owned);
                return Err(error_for_status(status.as_u16(), retry_after.as_deref()));
            }
            let (producer, stream) = open_stream(&cancel);
            tokio::spawn(read_events(response, producer));
            Ok(Box::pin(stream) as EventStream)
        })
    }
}

/// Reads the response until its last event, sending each token as it's parsed. Stops when the stream is cancelled,
/// when the response ends, or after the first error.
async fn read_events(response: reqwest::Response, out: Producer) {
    let mut chunks = response.bytes_stream();
    let mut decoder = SseDecoder::new();
    let mut usage = StreamUsage::default();
    loop {
        let next = tokio::select! {
            biased;
            _ = out.stopped() => return,
            chunk = chunks.next() => chunk,
        };
        let bytes = match next {
            Some(Ok(bytes)) => bytes,
            Some(Err(err)) => {
                out.send(Err(network_error(err))).await;
                return;
            }
            None => {
                out.send(Err(AiError::Protocol(
                    "the stream ended before the response finished".to_string(),
                )))
                .await;
                return;
            }
        };
        let events = match decoder.push(&bytes) {
            Ok(events) => events,
            Err(err) => {
                out.send(Err(err)).await;
                return;
            }
        };
        for event in events {
            match to_chat_event(&event, &mut usage) {
                Ok(None) => {}
                Ok(Some(chat)) => {
                    let finished = matches!(chat, ChatEvent::Finished { .. });
                    if !out.send(Ok(chat)).await || finished {
                        return;
                    }
                }
                Err(err) => {
                    out.send(Err(err)).await;
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    // Not `use super::*`: gpui's `test` macro would shadow `#[test]`.
    use std::time::Duration;

    use super::{
        StreamUsage, error_for_kind, error_for_status, parse_retry_after, request_body,
        to_chat_event,
    };
    use crate::{
        error::AiError,
        sse::SseDecoder,
        types::{ChatEvent, ChatMessage, ChatRequest, Usage},
    };

    const STREAM: &str = include_str!("../tests/fixtures/anthropic_stream.sse");
    const ERROR: &str = include_str!("../tests/fixtures/anthropic_error.sse");

    /// Runs a fixture through the decoder and the parser, in chunks of `size` bytes.
    fn read(fixture: &str, size: usize) -> Result<Vec<ChatEvent>, AiError> {
        let mut decoder = SseDecoder::new();
        let mut usage = StreamUsage::default();
        let mut events = Vec::new();
        for chunk in fixture.as_bytes().chunks(size) {
            for sse in decoder.push(chunk)? {
                if let Some(event) = to_chat_event(&sse, &mut usage)? {
                    events.push(event);
                }
            }
        }
        Ok(events)
    }

    fn tokens(events: &[ChatEvent]) -> String {
        events
            .iter()
            .filter_map(|event| match event {
                ChatEvent::Token(text) => Some(text.as_str()),
                ChatEvent::Finished { .. } => None,
            })
            .collect()
    }

    #[test]
    fn a_saved_stream_gives_its_text_and_its_usage() {
        let events = read(STREAM, 4096).unwrap();
        assert_eq!(tokens(&events), "Hello, café 😀");
        assert_eq!(
            events.last(),
            Some(&ChatEvent::Finished {
                usage: Some(Usage {
                    input_tokens: 12,
                    output_tokens: 7
                })
            })
        );
    }

    #[test]
    fn the_same_stream_read_one_byte_at_a_time_gives_the_same_events() {
        // Chunks this small split lines, JSON, and the multi-byte characters inside them.
        assert_eq!(read(STREAM, 1).unwrap(), read(STREAM, 4096).unwrap());
    }

    #[test]
    fn crlf_line_endings_give_the_same_events() {
        let crlf = STREAM.replace('\n', "\r\n");
        assert_eq!(read(&crlf, 7).unwrap(), read(STREAM, 4096).unwrap());
    }

    #[test]
    fn an_in_stream_overload_is_an_error_after_the_text_that_arrived_before_it() {
        let mut decoder = SseDecoder::new();
        let mut usage = StreamUsage::default();
        let mut events = Vec::new();
        let mut error = None;
        for sse in decoder.push(ERROR.as_bytes()).unwrap() {
            match to_chat_event(&sse, &mut usage) {
                Ok(Some(event)) => events.push(event),
                Ok(None) => {}
                Err(err) => error = Some(err),
            }
        }
        assert_eq!(tokens(&events), "Partial");
        assert_eq!(error, Some(AiError::Unavailable { status: 529 }));
    }

    #[test]
    fn an_event_whose_data_isnt_json_is_a_protocol_error_that_doesnt_echo_it() {
        let sse = crate::sse::SseEvent {
            event: Some("content_block_delta".to_string()),
            data: "secret conversation text".to_string(),
        };
        let err = to_chat_event(&sse, &mut StreamUsage::default()).unwrap_err();
        assert!(matches!(err, AiError::Protocol(_)));
        assert!(!err.to_string().contains("secret"));
    }

    #[test]
    fn statuses_map_to_the_errors_the_ui_tells_apart() {
        assert_eq!(error_for_status(401, None), AiError::AuthFailed);
        assert_eq!(error_for_status(403, None), AiError::AuthFailed);
        assert_eq!(
            error_for_status(429, Some("12")),
            AiError::RateLimited {
                retry_after: Some(Duration::from_secs(12))
            }
        );
        assert_eq!(
            error_for_status(429, None),
            AiError::RateLimited { retry_after: None }
        );
        assert_eq!(
            error_for_status(500, None),
            AiError::Unavailable { status: 500 }
        );
        assert_eq!(
            error_for_status(529, None),
            AiError::Unavailable { status: 529 }
        );
        assert_eq!(
            error_for_status(400, None),
            AiError::Rejected { status: 400 }
        );
        assert_eq!(
            error_for_status(404, None),
            AiError::Rejected { status: 404 }
        );
    }

    #[test]
    fn a_retry_after_that_isnt_a_number_of_seconds_is_ignored_not_guessed() {
        assert_eq!(parse_retry_after(" 7 "), Some(Duration::from_secs(7)));
        assert_eq!(parse_retry_after("Wed, 21 Oct 2026 07:28:00 GMT"), None);
        assert_eq!(parse_retry_after(""), None);
    }

    #[test]
    fn in_stream_error_types_map_to_the_matching_errors() {
        assert_eq!(error_for_kind("authentication_error"), AiError::AuthFailed);
        assert_eq!(
            error_for_kind("rate_limit_error"),
            AiError::RateLimited { retry_after: None }
        );
        assert!(matches!(
            error_for_kind("invalid_request_error"),
            AiError::Protocol(_)
        ));
    }

    #[test]
    fn the_request_body_asks_for_a_stream_and_keeps_roles_and_the_system_prompt() {
        let request = ChatRequest {
            model: "claude-sonnet-5-5".to_string(),
            system: Some("Be brief.".to_string()),
            messages: vec![ChatMessage::user("hi"), ChatMessage::assistant("hello")],
            max_tokens: 256,
        };
        let body = request_body(&request);
        assert_eq!(body["stream"], true);
        assert_eq!(body["model"], "claude-sonnet-5-5");
        assert_eq!(body["max_tokens"], 256);
        assert_eq!(body["system"], "Be brief.");
        assert_eq!(body["messages"][0]["role"], "user");
        assert_eq!(body["messages"][1]["role"], "assistant");
    }

    #[test]
    fn the_system_field_is_left_out_when_there_is_no_system_prompt() {
        let request = ChatRequest {
            model: "m".to_string(),
            system: None,
            messages: Vec::new(),
            max_tokens: 1,
        };
        assert!(request_body(&request).get("system").is_none());
    }
}
