//! The Anthropic API as an agent. The model is offered the tools, and each time it asks for some, they are run here and
//! their answers go back in the next request, until it answers in words. Requests use prompt caching on the
//! instructions and the tool definitions, which are the same for every round, so a long investigation only pays for
//! them once.
//!
//! The wire is behind [`Messages`], so the loop is tested with a script instead of a network.

use std::{collections::VecDeque, sync::Arc};

use futures_util::{
    Stream, StreamExt,
    future::{BoxFuture, join_all},
    stream::{self, BoxStream},
};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use super::{MAX_ROUNDS, ToolHost, ToolReply, describe_tool_use};
use crate::{
    AiError, ApiKey, ChatEvent, ChatRequest, Usage,
    anthropic::{API_VERSION, DEFAULT_BASE_URL, error_for_kind, error_for_status, network_error},
    provider::{ChatProvider, EventStream, Producer, open_stream},
    sse::{SseDecoder, SseEvent},
    types::ChatRole,
};

/// What the wire has to do: send one request, and give back its events as they arrive.
pub(crate) trait Messages: Send + Sync {
    fn send<'a>(
        &'a self,
        body: Value,
        cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<BoxStream<'static, Result<SseEvent, AiError>>, AiError>>;
}

/// The Messages API over HTTPS.
struct Https {
    key: ApiKey,
    client: reqwest::Client,
    base_url: String,
}

/// Turns a response body into events, one at a time, as the bytes arrive.
fn events_of<S, B>(chunks: S) -> BoxStream<'static, Result<SseEvent, AiError>>
where
    S: Stream<Item = reqwest::Result<B>> + Send + Unpin + 'static,
    B: AsRef<[u8]> + Send + 'static,
{
    struct Reading<S> {
        chunks: S,
        decoder: SseDecoder,
        queue: VecDeque<SseEvent>,
        done: bool,
    }
    stream::unfold(
        Reading {
            chunks,
            decoder: SseDecoder::new(),
            queue: VecDeque::new(),
            done: false,
        },
        |mut state| async move {
            loop {
                if let Some(event) = state.queue.pop_front() {
                    return Some((Ok(event), state));
                }
                if state.done {
                    return None;
                }
                match state.chunks.next().await {
                    Some(Ok(bytes)) => match state.decoder.push(bytes.as_ref()) {
                        Ok(events) => state.queue.extend(events),
                        Err(err) => {
                            state.done = true;
                            return Some((Err(err), state));
                        }
                    },
                    Some(Err(err)) => {
                        state.done = true;
                        return Some((Err(network_error(err)), state));
                    }
                    None => state.done = true,
                }
            }
        },
    )
    .boxed()
}

impl Messages for Https {
    fn send<'a>(
        &'a self,
        body: Value,
        cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<BoxStream<'static, Result<SseEvent, AiError>>, AiError>> {
        Box::pin(async move {
            let request = self
                .client
                .post(format!("{}/v1/messages", self.base_url))
                .header("x-api-key", self.key.expose())
                .header("anthropic-version", API_VERSION)
                .json(&body)
                .send();
            let response = tokio::select! {
                biased;
                _ = cancel.cancelled() => return Err(AiError::Cancelled),
                sent = request => sent.map_err(network_error)?,
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
            Ok(events_of(Box::pin(response.bytes_stream())))
        })
    }
}

/// The Anthropic API with tools.
pub struct AnthropicAgent {
    wire: Arc<dyn Messages>,
    host: Arc<dyn ToolHost>,
}

impl AnthropicAgent {
    pub fn new(key: ApiKey, host: Arc<dyn ToolHost>) -> Self {
        Self {
            wire: Arc::new(Https {
                key,
                client: reqwest::Client::new(),
                base_url: DEFAULT_BASE_URL.to_string(),
            }),
            host,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_wire(wire: Arc<dyn Messages>, host: Arc<dyn ToolHost>) -> Self {
        Self { wire, host }
    }
}

impl ChatProvider for AnthropicAgent {
    fn stream_chat(
        &self,
        request: ChatRequest,
        cancel: CancellationToken,
    ) -> BoxFuture<'_, Result<EventStream, AiError>> {
        Box::pin(async move {
            let (producer, stream) = open_stream(&cancel);
            let (wire, host) = (Arc::clone(&self.wire), Arc::clone(&self.host));
            tokio::spawn(async move {
                let outcome = run(wire.as_ref(), host, request, &cancel, &producer).await;
                match outcome {
                    Ok(usage) => {
                        producer
                            .send(Ok(ChatEvent::Finished { usage: Some(usage) }))
                            .await;
                    }
                    Err(AiError::Cancelled) => {}
                    Err(err) => {
                        producer.send(Err(err)).await;
                    }
                }
            });
            Ok(Box::pin(stream) as EventStream)
        })
    }
}

/// One content block of a response.
#[derive(Debug)]
enum Block {
    Text(String),
    ToolUse {
        id: String,
        name: String,
        input: String,
    },
}

/// One response, read to its end.
#[derive(Debug, Default)]
struct Turn {
    blocks: Vec<Block>,
    stop_reason: Option<String>,
    usage: Usage,
}

/// Reads one response's events. Text goes to `out` as it arrives. Everything else is kept for the caller.
async fn read_turn(
    mut events: BoxStream<'static, Result<SseEvent, AiError>>,
    out: &Producer,
) -> Result<Turn, AiError> {
    let mut turn = Turn::default();
    loop {
        let next = tokio::select! {
            biased;
            _ = out.stopped() => return Err(AiError::Cancelled),
            next = events.next() => next,
        };
        let Some(event) = next else {
            return Err(AiError::Protocol(
                "the response ended before it finished".to_string(),
            ));
        };
        let event = event?;
        let value: Value = serde_json::from_str(&event.data)
            .map_err(|_| AiError::Protocol("an event's data was not JSON".to_string()))?;
        let kind = value["type"]
            .as_str()
            .or(event.event.as_deref())
            .unwrap_or("");
        match kind {
            "message_start" => {
                turn.usage.input_tokens = count(&value["message"]["usage"]["input_tokens"]);
            }
            "content_block_start" => {
                let block = &value["content_block"];
                match block["type"].as_str() {
                    Some("text") => turn.blocks.push(Block::Text(String::new())),
                    Some("tool_use") => turn.blocks.push(Block::ToolUse {
                        id: block["id"].as_str().unwrap_or_default().to_string(),
                        name: block["name"].as_str().unwrap_or_default().to_string(),
                        input: String::new(),
                    }),
                    _ => {}
                }
            }
            "content_block_delta" => {
                match (value["delta"]["type"].as_str(), turn.blocks.last_mut()) {
                    (Some("text_delta"), Some(Block::Text(text))) => {
                        if let Some(piece) = value["delta"]["text"].as_str() {
                            text.push_str(piece);
                            if !out.send(Ok(ChatEvent::Token(piece.to_string()))).await {
                                return Err(AiError::Cancelled);
                            }
                        }
                    }
                    (Some("input_json_delta"), Some(Block::ToolUse { input, .. })) => {
                        if let Some(piece) = value["delta"]["partial_json"].as_str() {
                            input.push_str(piece);
                        }
                    }
                    _ => {}
                }
            }
            "message_delta" => {
                if let Some(reason) = value["delta"]["stop_reason"].as_str() {
                    turn.stop_reason = Some(reason.to_string());
                }
                turn.usage.output_tokens = count(&value["usage"]["output_tokens"]);
            }
            "message_stop" => return Ok(turn),
            "error" => {
                return Err(error_for_kind(
                    value["error"]["type"].as_str().unwrap_or(""),
                ));
            }
            _ => {}
        }
    }
}

fn count(value: &Value) -> u32 {
    value
        .as_u64()
        .and_then(|number| u32::try_from(number).ok())
        .unwrap_or(0)
}

/// The tools as the API wants them. The last carries the cache marker, which caches every tool before it too.
fn tools_json(host: &dyn ToolHost) -> Vec<Value> {
    let mut tools: Vec<Value> = host
        .specs()
        .into_iter()
        .map(|spec| json!({ "name": spec.name, "description": spec.description, "input_schema": spec.schema }))
        .collect();
    if let Some(last) = tools.last_mut() {
        last["cache_control"] = json!({ "type": "ephemeral" });
    }
    tools
}

/// The request body for one round. The last round offers no tools, so the model has to answer.
fn body_for(request: &ChatRequest, messages: &[Value], tools: &[Value], with_tools: bool) -> Value {
    let mut body = json!({
        "model": request.model,
        "max_tokens": request.max_tokens,
        "messages": messages,
        "stream": true,
    });
    if let Some(system) = &request.system {
        body["system"] =
            json!([{ "type": "text", "text": system, "cache_control": { "type": "ephemeral" } }]);
    }
    if with_tools && !tools.is_empty() {
        body["tools"] = json!(tools);
    }
    body
}

/// The loop: ask, run the tools it asks for, ask again, until it answers.
async fn run(
    wire: &dyn Messages,
    host: Arc<dyn ToolHost>,
    request: ChatRequest,
    cancel: &CancellationToken,
    out: &Producer,
) -> Result<Usage, AiError> {
    let mut messages: Vec<Value> = request
        .messages
        .iter()
        .map(|message| {
            json!({
                "role": match message.role { ChatRole::User => "user", ChatRole::Assistant => "assistant" },
                "content": message.content,
            })
        })
        .collect();
    let tools = tools_json(host.as_ref());
    let mut total = Usage::default();

    for round in 0..=MAX_ROUNDS {
        let with_tools = round < MAX_ROUNDS;
        let events = wire
            .send(body_for(&request, &messages, &tools, with_tools), cancel)
            .await?;
        let turn = read_turn(events, out).await?;
        total.input_tokens += turn.usage.input_tokens;
        total.output_tokens += turn.usage.output_tokens;

        let asked: Vec<(&str, &str, &str)> = turn
            .blocks
            .iter()
            .filter_map(|block| match block {
                Block::ToolUse { id, name, input } => {
                    Some((id.as_str(), name.as_str(), input.as_str()))
                }
                Block::Text(_) => None,
            })
            .collect();
        if turn.stop_reason.as_deref() != Some("tool_use") || asked.is_empty() || !with_tools {
            return Ok(total);
        }

        // What the model said, then what it asked for, goes back as its own turn.
        let mut assistant: Vec<Value> = Vec::new();
        for block in &turn.blocks {
            match block {
                Block::Text(text) if !text.is_empty() => {
                    assistant.push(json!({ "type": "text", "text": text }))
                }
                Block::ToolUse { id, name, input } => assistant.push(json!({
                    "type": "tool_use",
                    "id": id,
                    "name": name,
                    "input": serde_json::from_str::<Value>(input).unwrap_or_else(|_| json!({})),
                })),
                Block::Text(_) => {}
            }
        }
        messages.push(json!({ "role": "assistant", "content": assistant }));

        let calls: Vec<(String, String, Value)> = asked
            .iter()
            .map(|(id, name, input)| {
                (
                    id.to_string(),
                    name.to_string(),
                    serde_json::from_str::<Value>(input).unwrap_or_else(|_| json!({})),
                )
            })
            .collect();
        for (_, name, input) in &calls {
            if !out
                .send(Ok(ChatEvent::Step(describe_tool_use(name, input))))
                .await
            {
                return Err(AiError::Cancelled);
            }
        }
        // The tools run side by side, off the async threads, since they read files and databases.
        let replies: Vec<ToolReply> = join_all(calls.iter().map(|(_, name, input)| {
            let (host, name, input) = (Arc::clone(&host), name.clone(), input.clone());
            async move {
                tokio::task::spawn_blocking(move || host.call(&name, &input))
                    .await
                    .unwrap_or_else(|_| ToolReply {
                        text: "The tool stopped unexpectedly.".to_string(),
                        sources: Vec::new(),
                        is_error: true,
                    })
            }
        }))
        .await;

        let mut results: Vec<Value> = Vec::new();
        for ((id, _, _), reply) in calls.iter().zip(replies) {
            for source in reply.sources {
                if !out.send(Ok(ChatEvent::Source(source))).await {
                    return Err(AiError::Cancelled);
                }
            }
            let mut result =
                json!({ "type": "tool_result", "tool_use_id": id, "content": reply.text });
            if reply.is_error {
                result["is_error"] = json!(true);
            }
            results.push(result);
        }
        if round + 1 == MAX_ROUNDS {
            results.push(json!({
                "type": "text",
                "text": "That was the last lookup you may make. Answer now with what you found, and say what you could not check.",
            }));
        }
        messages.push(json!({ "role": "user", "content": results }));
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use futures_util::StreamExt;

    use super::*;
    use crate::{
        agent::ToolSpec,
        generation::{Update, run_with},
        types::{ChatMessage, Source},
    };

    /// One scripted response: its events, as the server would send them.
    fn response(events: &[(&str, Value)]) -> Vec<SseEvent> {
        events
            .iter()
            .map(|(kind, data)| SseEvent {
                event: Some((*kind).to_string()),
                data: data.to_string(),
            })
            .collect()
    }

    fn tool_round(id: &str, name: &str, input_pieces: &[&str]) -> Vec<SseEvent> {
        let mut events = vec![
            (
                "message_start",
                json!({"type": "message_start", "message": {"usage": {"input_tokens": 100}}}),
            ),
            (
                "content_block_start",
                json!({"type": "content_block_start", "index": 0, "content_block": {"type": "tool_use", "id": id, "name": name, "input": {}}}),
            ),
        ];
        for piece in input_pieces {
            events.push(("content_block_delta", json!({"type": "content_block_delta", "index": 0, "delta": {"type": "input_json_delta", "partial_json": piece}})));
        }
        events.push((
            "content_block_stop",
            json!({"type": "content_block_stop", "index": 0}),
        ));
        events.push(("message_delta", json!({"type": "message_delta", "delta": {"stop_reason": "tool_use"}, "usage": {"output_tokens": 20}})));
        events.push(("message_stop", json!({"type": "message_stop"})));
        response(&events)
    }

    fn answer_round(pieces: &[&str]) -> Vec<SseEvent> {
        let mut events = vec![
            (
                "message_start",
                json!({"type": "message_start", "message": {"usage": {"input_tokens": 200}}}),
            ),
            (
                "content_block_start",
                json!({"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}),
            ),
        ];
        for piece in pieces {
            events.push(("content_block_delta", json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": piece}})));
        }
        events.push((
            "content_block_stop",
            json!({"type": "content_block_stop", "index": 0}),
        ));
        events.push(("message_delta", json!({"type": "message_delta", "delta": {"stop_reason": "end_turn"}, "usage": {"output_tokens": 30}})));
        events.push(("message_stop", json!({"type": "message_stop"})));
        response(&events)
    }

    /// A wire that plays back scripted responses and keeps the bodies it was sent.
    struct Script {
        responses: Mutex<VecDeque<Vec<SseEvent>>>,
        bodies: Arc<Mutex<Vec<Value>>>,
    }

    impl Messages for Script {
        fn send<'a>(
            &'a self,
            body: Value,
            _cancel: &'a CancellationToken,
        ) -> BoxFuture<'a, Result<BoxStream<'static, Result<SseEvent, AiError>>, AiError>> {
            self.bodies.lock().unwrap().push(body);
            let next = self.responses.lock().unwrap().pop_front();
            Box::pin(async move {
                let events =
                    next.ok_or_else(|| AiError::Protocol("the script ran out".to_string()))?;
                Ok(stream::iter(events.into_iter().map(Ok)).boxed())
            })
        }
    }

    struct Tools {
        calls: Mutex<Vec<(String, Value)>>,
    }

    impl ToolHost for Tools {
        fn specs(&self) -> Vec<ToolSpec> {
            vec![
                ToolSpec {
                    name: "search".into(),
                    description: "Search.".into(),
                    schema: json!({"type": "object"}),
                },
                ToolSpec {
                    name: "read_file".into(),
                    description: "Read.".into(),
                    schema: json!({"type": "object"}),
                },
            ]
        }

        fn call(&self, name: &str, input: &Value) -> ToolReply {
            self.calls
                .lock()
                .unwrap()
                .push((name.to_string(), input.clone()));
            if name == "read_file" {
                return ToolReply {
                    text: "No such file.".into(),
                    sources: Vec::new(),
                    is_error: true,
                };
            }
            ToolReply {
                text: "[1] src/auth.rs:1-3".into(),
                sources: vec![Source {
                    kind: "code".into(),
                    reference: "src/auth.rs".into(),
                    label: "fn verify".into(),
                    start_line: 1,
                    end_line: 3,
                }],
                is_error: false,
            }
        }
    }

    struct Run {
        updates: Vec<Update>,
        bodies: Vec<Value>,
        calls: Vec<(String, Value)>,
    }

    fn run_script(rounds: Vec<Vec<SseEvent>>) -> Run {
        let bodies = Arc::new(Mutex::new(Vec::new()));
        let wire = Arc::new(Script {
            responses: Mutex::new(rounds.into()),
            bodies: Arc::clone(&bodies),
        });
        let tools = Arc::new(Tools {
            calls: Mutex::new(Vec::new()),
        });
        let agent = AnthropicAgent::with_wire(wire, tools.clone());
        let request = ChatRequest {
            model: "claude-sonnet-5-5".into(),
            system: Some("Be grounded.".into()),
            messages: vec![ChatMessage::user("Where is the password checked?")],
            max_tokens: 1000,
        };
        let updates = Mutex::new(Vec::new());
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(run_with(
            &agent,
            request,
            CancellationToken::new(),
            |update| updates.lock().unwrap().push(update),
        ));
        let bodies = bodies.lock().unwrap().clone();
        let calls = tools.calls.lock().unwrap().clone();
        Run {
            updates: updates.into_inner().unwrap(),
            bodies,
            calls,
        }
    }

    #[test]
    fn a_question_that_needs_a_lookup_runs_the_tool_then_answers_with_its_sources() {
        let run = run_script(vec![
            tool_round(
                "toolu_1",
                "search",
                &["{\"query\":", "\"verify password\"}"],
            ),
            answer_round(&["It is in ", "src/auth.rs."]),
        ]);
        assert_eq!(
            run.updates,
            [
                Update::Step("Searching for \"verify password\"".into()),
                Update::Source(Source {
                    kind: "code".into(),
                    reference: "src/auth.rs".into(),
                    label: "fn verify".into(),
                    start_line: 1,
                    end_line: 3
                }),
                Update::Text("It is in ".into()),
                Update::Text("src/auth.rs.".into()),
                Update::Finished("It is in src/auth.rs.".into()),
            ]
        );
        assert_eq!(
            run.calls,
            [("search".to_string(), json!({"query": "verify password"}))]
        );
    }

    #[test]
    fn the_tool_answer_goes_back_in_the_next_request_with_what_the_model_asked() {
        let run = run_script(vec![
            tool_round("toolu_1", "search", &["{\"query\":\"x\"}"]),
            answer_round(&["Done."]),
        ]);
        let second = &run.bodies[1]["messages"];
        assert_eq!(second.as_array().unwrap().len(), 3);
        assert_eq!(second[1]["role"], "assistant");
        assert_eq!(second[1]["content"][0]["type"], "tool_use");
        assert_eq!(second[1]["content"][0]["id"], "toolu_1");
        assert_eq!(second[1]["content"][0]["input"], json!({"query": "x"}));
        assert_eq!(second[2]["role"], "user");
        assert_eq!(second[2]["content"][0]["type"], "tool_result");
        assert_eq!(second[2]["content"][0]["tool_use_id"], "toolu_1");
        assert_eq!(second[2]["content"][0]["content"], "[1] src/auth.rs:1-3");
        assert!(second[2]["content"][0].get("is_error").is_none());
    }

    #[test]
    fn the_instructions_and_the_tools_are_marked_for_caching_in_every_request() {
        let run = run_script(vec![
            tool_round("t", "search", &["{}"]),
            answer_round(&["ok"]),
        ]);
        for body in &run.bodies {
            assert_eq!(body["system"][0]["cache_control"]["type"], "ephemeral");
            let tools = body["tools"].as_array().unwrap();
            assert_eq!(tools.len(), 2);
            assert!(tools[0].get("cache_control").is_none());
            assert_eq!(tools[1]["cache_control"]["type"], "ephemeral");
            assert_eq!(body["stream"], true);
        }
    }

    #[test]
    fn a_failed_tool_call_is_marked_as_an_error_so_the_model_can_try_something_else() {
        let run = run_script(vec![
            tool_round("t", "read_file", &["{\"path\":\"nope\"}"]),
            answer_round(&["I could not find it."]),
        ]);
        assert_eq!(run.bodies[1]["messages"][2]["content"][0]["is_error"], true);
        assert!(matches!(run.updates.last(), Some(Update::Finished(_))));
    }

    #[test]
    fn several_tool_calls_in_one_turn_are_all_answered_in_one_message() {
        let mut first = tool_round("a", "search", &["{\"query\":\"one\"}"]);
        // A second tool call in the same response.
        first.splice(
            4..4,
            response(&[
                ("content_block_start", json!({"type": "content_block_start", "index": 1, "content_block": {"type": "tool_use", "id": "b", "name": "search", "input": {}}})),
                ("content_block_delta", json!({"type": "content_block_delta", "index": 1, "delta": {"type": "input_json_delta", "partial_json": "{\"query\":\"two\"}"}})),
                ("content_block_stop", json!({"type": "content_block_stop", "index": 1})),
            ]),
        );
        let run = run_script(vec![first, answer_round(&["Both."])]);
        let results = &run.bodies[1]["messages"][2]["content"];
        assert_eq!(results.as_array().unwrap().len(), 2);
        assert_eq!(results[0]["tool_use_id"], "a");
        assert_eq!(results[1]["tool_use_id"], "b");
        assert_eq!(run.calls.len(), 2);
    }

    #[test]
    fn a_model_that_keeps_asking_is_made_to_answer_after_the_most_rounds() {
        let mut rounds: Vec<Vec<SseEvent>> = (0..MAX_ROUNDS)
            .map(|n| tool_round(&format!("t{n}"), "search", &["{\"query\":\"again\"}"]))
            .collect();
        rounds.push(answer_round(&["Here is what I found."]));
        let run = run_script(rounds);
        assert_eq!(run.calls.len(), MAX_ROUNDS);
        let last = run.bodies.last().unwrap();
        assert!(
            last.get("tools").is_none(),
            "the final round offers no tools"
        );
        let closing = last["messages"].as_array().unwrap().last().unwrap()["content"]
            .as_array()
            .unwrap()
            .last()
            .unwrap()
            .clone();
        assert!(closing["text"].as_str().unwrap().contains("last lookup"));
        assert!(
            matches!(run.updates.last(), Some(Update::Finished(text)) if text == "Here is what I found.")
        );
    }

    #[test]
    fn an_error_event_fails_the_run_with_the_providers_reason() {
        let run = run_script(vec![response(&[(
            "error",
            json!({"type": "error", "error": {"type": "overloaded_error"}}),
        )])]);
        assert_eq!(
            run.updates,
            [Update::Failed(AiError::Unavailable { status: 529 })]
        );
    }

    #[test]
    fn a_response_that_ends_early_is_a_failure_not_an_answer() {
        let run = run_script(vec![response(&[(
            "message_start",
            json!({"type": "message_start", "message": {"usage": {"input_tokens": 1}}}),
        )])]);
        assert!(matches!(
            run.updates.as_slice(),
            [Update::Failed(AiError::Protocol(_))]
        ));
    }

    #[test]
    fn usage_is_added_up_across_rounds() {
        let bodies = Arc::new(Mutex::new(Vec::new()));
        let wire = Arc::new(Script {
            responses: Mutex::new(
                vec![tool_round("t", "search", &["{}"]), answer_round(&["ok"])].into(),
            ),
            bodies,
        });
        let agent = AnthropicAgent::with_wire(
            wire,
            Arc::new(Tools {
                calls: Mutex::new(Vec::new()),
            }),
        );
        let request = ChatRequest {
            model: "m".into(),
            system: None,
            messages: vec![ChatMessage::user("q")],
            max_tokens: 10,
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let usage = runtime.block_on(async {
            let mut stream = agent
                .stream_chat(request, CancellationToken::new())
                .await
                .unwrap();
            let mut finished = None;
            while let Some(event) = stream.next().await {
                if let Ok(ChatEvent::Finished { usage }) = event {
                    finished = usage;
                }
            }
            finished
        });
        assert_eq!(
            usage,
            Some(Usage {
                input_tokens: 300,
                output_tokens: 50
            })
        );
    }
}
