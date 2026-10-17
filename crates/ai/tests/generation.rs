//! The runner that the wizard uses: what it reports, in what order, and what it stops reporting after a cancel.

use std::{sync::Mutex, time::Duration};

use looji_ai::{
    AiError, CancellationToken, ChatMessage, ChatRequest,
    fake::{FakeProvider, Script, Step},
    generation::{Update, run_with},
};

fn request() -> ChatRequest {
    ChatRequest {
        model: "claude-sonnet-5-5".to_string(),
        system: None,
        messages: vec![ChatMessage::user("plan")],
        max_tokens: 64,
    }
}

async fn run_collecting(provider: &FakeProvider, cancel: CancellationToken) -> Vec<Update> {
    let seen = Mutex::new(Vec::new());
    run_with(provider, request(), cancel, |update| {
        seen.lock().unwrap().push(update)
    })
    .await;
    seen.into_inner().unwrap()
}

#[tokio::test]
async fn text_is_reported_as_it_arrives_then_the_whole_response_finishes() {
    let provider = FakeProvider::new();
    provider.push(Script::tokens(["{\"spec\"", ":\"x\"}"]));

    let updates = run_collecting(&provider, CancellationToken::new()).await;

    assert_eq!(
        updates,
        vec![
            Update::Text("{\"spec\"".to_string()),
            Update::Text(":\"x\"}".to_string()),
            Update::Finished("{\"spec\":\"x\"}".to_string()),
        ]
    );
}

#[tokio::test]
async fn a_request_refused_before_streaming_is_reported_as_failed() {
    let provider = FakeProvider::new();
    provider.push(Script::refused(AiError::AuthFailed));

    let updates = run_collecting(&provider, CancellationToken::new()).await;

    assert_eq!(updates, vec![Update::Failed(AiError::AuthFailed)]);
}

#[tokio::test]
async fn an_error_in_the_stream_keeps_the_text_before_it_and_reports_the_error() {
    let provider = FakeProvider::new();
    provider.push(Script::new(vec![
        Step::Token("partial".to_string()),
        Step::Fail(AiError::Unavailable { status: 529 }),
    ]));

    let updates = run_collecting(&provider, CancellationToken::new()).await;

    assert_eq!(
        updates,
        vec![
            Update::Text("partial".to_string()),
            Update::Failed(AiError::Unavailable { status: 529 }),
        ]
    );
}

#[tokio::test]
async fn a_stream_that_ends_without_finishing_is_a_failure() {
    let provider = FakeProvider::new();
    provider.push(Script::new(vec![Step::Token("cut".to_string())]));

    let updates = run_collecting(&provider, CancellationToken::new()).await;

    assert!(matches!(
        updates.last(),
        Some(Update::Failed(AiError::Protocol(_)))
    ));
}

#[tokio::test]
async fn nothing_is_reported_after_a_cancel() {
    let provider = FakeProvider::new();
    provider.push(Script::ticking(1000, Duration::from_millis(5)));
    let cancel = CancellationToken::new();
    let seen = std::sync::Arc::new(Mutex::new(Vec::new()));
    let sink = std::sync::Arc::clone(&seen);

    let canceller = cancel.clone();
    let run = run_with(&provider, request(), cancel, move |update| {
        let mut seen = sink.lock().unwrap();
        seen.push(update);
        if seen.len() == 3 {
            canceller.cancel();
        }
    });
    tokio::time::timeout(Duration::from_secs(2), run)
        .await
        .expect("a cancelled run stops");

    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 3, "updates after the cancel are not reported");
    assert!(seen.iter().all(|update| matches!(update, Update::Text(_))));
}
