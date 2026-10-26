//! Runs one request to completion and reports what happens as it goes. This is the piece the UI spawns on the AI
//! runtime, so the UI only receives updates and never touches the network.

use futures_util::StreamExt;
use tokio_util::sync::CancellationToken;

use crate::{
    error::AiError,
    provider::ChatProvider,
    types::{ChatEvent, ChatRequest},
};

/// What a run reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Update {
    /// Text that just arrived.
    Text(String),
    /// What an agent has started doing, between pieces of text.
    Step(String),
    /// Something the agent read, which the answer can cite.
    Source(crate::types::Source),
    /// The response finished. The text is the whole response, so the caller can check it against what it received.
    Finished(String),
    /// The run couldn't finish. A cancelled run reports nothing, since the caller already knows.
    Failed(AiError),
}

/// Sends `request` through the provider `connect` returns, and reports each update through `send`.
///
/// Stops quietly when `cancel` is cancelled. Nothing is reported after a cancel, so a late chunk can't reach
/// the caller.
pub async fn run_request(
    provider: crate::Provider,
    request: ChatRequest,
    cancel: CancellationToken,
    send: impl Fn(Update) + Send,
) {
    let provider = match crate::connect(provider) {
        Ok(provider) => provider,
        Err(err) => return send(Update::Failed(err)),
    };
    run_with(provider.as_ref(), request, cancel, send).await;
}

/// The same as `run_request`, for a provider the caller already has. Tests use this with a fake provider.
pub async fn run_with(
    provider: &dyn ChatProvider,
    request: ChatRequest,
    cancel: CancellationToken,
    send: impl Fn(Update) + Send,
) {
    let mut stream = match provider.stream_chat(request, cancel.clone()).await {
        Ok(stream) => stream,
        Err(AiError::Cancelled) => return,
        Err(err) => return send(Update::Failed(err)),
    };
    let mut text = String::new();
    while let Some(event) = stream.next().await {
        if cancel.is_cancelled() {
            return;
        }
        match event {
            Ok(ChatEvent::Token(token)) => {
                text.push_str(&token);
                send(Update::Text(token));
            }
            Ok(ChatEvent::Step(step)) => send(Update::Step(step)),
            Ok(ChatEvent::Source(source)) => send(Update::Source(source)),
            Ok(ChatEvent::Finished { .. }) => return send(Update::Finished(text)),
            Err(AiError::Cancelled) => return,
            Err(err) => return send(Update::Failed(err)),
        }
    }
    send(Update::Failed(AiError::Protocol(
        "the stream ended before the response finished".to_string(),
    )));
}

/// The same as `run_with`, but returns the whole reply rather than reporting it as it arrives. For answers that are
/// only read once they're complete.
pub async fn complete_with(
    provider: &dyn ChatProvider,
    request: ChatRequest,
    cancel: CancellationToken,
) -> Result<String, AiError> {
    let outcome = std::sync::Mutex::new(None);
    run_with(provider, request, cancel, |update| match update {
        Update::Finished(text) => {
            *outcome.lock().expect("the outcome isn't poisoned") = Some(Ok(text))
        }
        Update::Failed(err) => {
            *outcome.lock().expect("the outcome isn't poisoned") = Some(Err(err))
        }
        Update::Text(_) | Update::Step(_) | Update::Source(_) => {}
    })
    .await;
    outcome
        .into_inner()
        .expect("the outcome isn't poisoned")
        .unwrap_or(Err(AiError::Cancelled))
}
