//! The fake provider's scripts, and the cancellation rules the streams follow. Every wait is a timeout on a channel
//! or a counter, never a fixed sleep, so these don't flake on a busy machine.

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use futures_util::StreamExt;
use looji_ai::{
    AiError, ChatEvent, ChatMessage, ChatProvider, ChatRequest, EventStream, Usage,
    fake::{FakeProvider, Script, Step, StopCount},
};
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

/// The longest a stream may take to stop delivering after a cancel.
const CANCEL_WINDOW: Duration = Duration::from_millis(100);
/// The longest any single wait may take before the test calls it a hang.
const PATIENCE: Duration = Duration::from_secs(2);
const TICK: Duration = Duration::from_millis(10);

fn request(text: &str) -> ChatRequest {
    ChatRequest {
        model: "claude-sonnet-5-5".to_string(),
        system: None,
        messages: vec![ChatMessage::user(text)],
        max_tokens: 64,
    }
}

async fn open(provider: &FakeProvider, cancel: &CancellationToken) -> EventStream {
    provider
        .stream_chat(request("hi"), cancel.clone())
        .await
        .expect("the fake accepts the request")
}

/// The next item, or a panic if nothing arrives within `PATIENCE`.
async fn next(stream: &mut EventStream) -> Option<Result<ChatEvent, AiError>> {
    timeout(PATIENCE, stream.next())
        .await
        .expect("an item arrives, or the stream ends, within the patience window")
}

/// Reads `count` tokens and returns their text.
async fn read_tokens(stream: &mut EventStream, count: usize) -> String {
    let mut text = String::new();
    for _ in 0..count {
        match next(stream).await {
            Some(Ok(ChatEvent::Token(token))) => text.push_str(&token),
            other => panic!("expected a token, got {other:?}"),
        }
    }
    text
}

#[tokio::test]
async fn scripts_play_back_in_the_order_they_were_queued() {
    let provider = FakeProvider::new();
    provider.push(Script::tokens(["Hel", "lo"]));
    provider.push(Script::tokens(["Bye"]));
    let cancel = CancellationToken::new();

    let mut first = open(&provider, &cancel).await;
    assert_eq!(read_tokens(&mut first, 2).await, "Hello");
    assert_eq!(
        next(&mut first).await,
        Some(Ok(ChatEvent::Finished { usage: None }))
    );
    assert_eq!(next(&mut first).await, None);

    let mut second = open(&provider, &cancel).await;
    assert_eq!(read_tokens(&mut second, 1).await, "Bye");
}

#[tokio::test]
async fn a_script_can_pause_fail_and_finish_with_usage() {
    let provider = FakeProvider::new();
    provider.push(Script::new(vec![
        Step::Token("partial".to_string()),
        Step::Pause(Duration::from_millis(20)),
        Step::Fail(AiError::RateLimited {
            retry_after: Some(Duration::from_secs(5)),
        }),
    ]));
    provider.push(Script::new(vec![Step::End(Some(Usage {
        input_tokens: 3,
        output_tokens: 1,
    }))]));
    let cancel = CancellationToken::new();

    let mut failing = open(&provider, &cancel).await;
    assert_eq!(read_tokens(&mut failing, 1).await, "partial");
    assert_eq!(
        next(&mut failing).await,
        Some(Err(AiError::RateLimited {
            retry_after: Some(Duration::from_secs(5))
        }))
    );
    assert_eq!(next(&mut failing).await, None);

    let mut finishing = open(&provider, &cancel).await;
    assert_eq!(
        next(&mut finishing).await,
        Some(Ok(ChatEvent::Finished {
            usage: Some(Usage {
                input_tokens: 3,
                output_tokens: 1
            })
        }))
    );
}

#[tokio::test]
async fn a_refusal_fails_the_request_before_any_stream_exists() {
    let provider = FakeProvider::new();
    provider.push(Script::refused(AiError::AuthFailed));
    let cancel = CancellationToken::new();

    let result = provider.stream_chat(request("hi"), cancel).await;

    assert!(matches!(result, Err(AiError::AuthFailed)));
}

#[tokio::test]
async fn running_out_of_scripts_is_an_error_not_a_hang() {
    let provider = FakeProvider::new();
    let result = provider
        .stream_chat(request("hi"), CancellationToken::new())
        .await;
    assert!(matches!(result, Err(AiError::Protocol(_))));
}

#[tokio::test]
async fn the_fake_records_every_request_it_receives() {
    let provider = FakeProvider::new();
    provider.push(Script::tokens(["ok"]));
    provider.push(Script::tokens(["ok"]));
    let cancel = CancellationToken::new();

    let _ = open(&provider, &cancel).await;
    let _ = provider
        .stream_chat(
            ChatRequest {
                model: "claude-opus-5-5".to_string(),
                system: Some("Be brief.".to_string()),
                messages: vec![ChatMessage::user("a"), ChatMessage::assistant("b")],
                max_tokens: 8,
            },
            cancel,
        )
        .await
        .unwrap();

    let requests = provider.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1].model, "claude-opus-5-5");
    assert_eq!(requests[1].system.as_deref(), Some("Be brief."));
    assert_eq!(requests[1].messages.len(), 2);
}

#[tokio::test]
async fn cancelling_stops_token_delivery_within_the_window_and_the_producer_stops() {
    let provider = FakeProvider::new();
    provider.push(Script::ticking(1000, TICK));
    let stops: Arc<StopCount> = provider.stops();
    let cancel = CancellationToken::new();
    let mut stream = open(&provider, &cancel).await;

    assert_eq!(read_tokens(&mut stream, 3).await, "t0 t1 t2 ");

    cancel.cancel();
    let cancelled_at = Instant::now();

    // The first item after the cancel is the distinct Cancelled error, and it arrives inside the window. Anything
    // else, such as a token, would mean delivery carried on past the cancel.
    let after = timeout(CANCEL_WINDOW, stream.next())
        .await
        .expect("the cancel is seen within the window");
    assert_eq!(after, Some(Err(AiError::Cancelled)));
    assert!(cancelled_at.elapsed() <= CANCEL_WINDOW);

    assert_eq!(
        timeout(CANCEL_WINDOW, stream.next())
            .await
            .expect("the stream ends within the window"),
        None,
        "the stream ends after the cancel and delivers nothing more"
    );
    assert!(
        stops.wait_until(1, CANCEL_WINDOW).await,
        "the producer task stops within the window"
    );
}

#[tokio::test]
async fn text_that_arrived_before_the_cancel_is_kept() {
    let provider = FakeProvider::new();
    provider.push(Script::ticking(1000, TICK));
    let cancel = CancellationToken::new();
    let mut stream = open(&provider, &cancel).await;

    let kept = read_tokens(&mut stream, 3).await;
    cancel.cancel();

    assert_eq!(kept, "t0 t1 t2 ");
    assert_eq!(next(&mut stream).await, Some(Err(AiError::Cancelled)));
}

#[tokio::test]
async fn cancelling_during_a_long_pause_stops_the_producer_without_waiting_for_the_pause() {
    let provider = FakeProvider::new();
    provider.push(Script::new(vec![Step::Pause(Duration::from_secs(60))]));
    let stops = provider.stops();
    let cancel = CancellationToken::new();
    let mut stream = open(&provider, &cancel).await;

    cancel.cancel();

    assert_eq!(next(&mut stream).await, Some(Err(AiError::Cancelled)));
    assert!(stops.wait_until(1, CANCEL_WINDOW).await);
}

#[tokio::test]
async fn a_cancel_before_the_first_read_delivers_no_tokens() {
    let provider = FakeProvider::new();
    provider.push(Script::tokens(["never", "seen"]));
    let cancel = CancellationToken::new();
    let mut stream = open(&provider, &cancel).await;

    cancel.cancel();

    assert_eq!(next(&mut stream).await, Some(Err(AiError::Cancelled)));
    assert_eq!(next(&mut stream).await, None);
}

#[tokio::test]
async fn dropping_the_stream_stops_the_producer_without_cancelling_the_callers_token() {
    let provider = FakeProvider::new();
    provider.push(Script::ticking(1000, TICK));
    let stops = provider.stops();
    let cancel = CancellationToken::new();
    let mut stream = open(&provider, &cancel).await;
    assert_eq!(read_tokens(&mut stream, 1).await, "t0 ");

    drop(stream);

    assert!(
        stops.wait_until(1, CANCEL_WINDOW).await,
        "dropping the stream stops the producer within the window"
    );
    assert!(
        !cancel.is_cancelled(),
        "dropping a stream cancels only that stream, not the caller's token"
    );
}

#[tokio::test]
async fn a_producer_that_finishes_on_its_own_is_counted_as_stopped_too() {
    let provider = FakeProvider::new();
    provider.push(Script::tokens(["done"]));
    let stops = provider.stops();
    let cancel = CancellationToken::new();
    let mut stream = open(&provider, &cancel).await;
    assert_eq!(read_tokens(&mut stream, 1).await, "done");
    assert_eq!(
        next(&mut stream).await,
        Some(Ok(ChatEvent::Finished { usage: None }))
    );

    assert!(stops.wait_until(1, PATIENCE).await);
}
