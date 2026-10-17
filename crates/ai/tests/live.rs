//! Calls the live Anthropic API. Ignored by default, since it spends tokens and needs a key.
//!
//! Run it with: `ANTHROPIC_API_KEY=... cargo test -p looji-ai --test live -- --ignored`

use looji_ai::{
    ApiKey, ChatEvent, ChatMessage, ChatProvider, ChatRequest, anthropic::AnthropicProvider,
    models::DEFAULT_MODEL,
};
use tokio_util::sync::CancellationToken;

#[test]
#[ignore = "calls the live API; set ANTHROPIC_API_KEY and run with --ignored"]
fn a_live_request_streams_text_and_finishes() {
    let key = std::env::var("ANTHROPIC_API_KEY").expect("ANTHROPIC_API_KEY is set");
    let key = ApiKey::new(key).expect("ANTHROPIC_API_KEY isn't blank");
    let provider = AnthropicProvider::new(key);

    looji_ai::runtime().block_on(async {
        let request = ChatRequest {
            model: DEFAULT_MODEL.to_string(),
            system: None,
            messages: vec![ChatMessage::user("Reply with the single word: pong")],
            max_tokens: 16,
        };
        let mut stream = provider
            .stream_chat(request, CancellationToken::new())
            .await
            .expect("the live API accepts the request");

        let mut text = String::new();
        let mut finished = false;
        while let Some(event) = futures_util::StreamExt::next(&mut stream).await {
            match event.expect("the live stream has no error") {
                ChatEvent::Token(token) => text.push_str(&token),
                ChatEvent::Finished { .. } => finished = true,
            }
        }
        assert!(finished, "the stream finishes");
        assert!(!text.trim().is_empty(), "the stream carries text");
    });
}
