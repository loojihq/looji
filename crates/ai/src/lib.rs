//! A provider-agnostic chat client. The rest of the app calls [`ChatProvider`] and never sees a vendor's types.
//!
//! The network work runs on a Tokio runtime that this crate owns (see [`runtime`]), so it never blocks the UI's
//! thread. Events come back through the stream, which the caller reads from wherever it likes.
//!
//! Nothing here depends on GPUI or SQLite. Only the UI should depend on this crate.

pub mod anthropic;
pub mod chat;
mod error;
pub mod fake;
pub mod generation;
mod key;
pub mod key_store;
pub mod models;
pub mod planning;
mod provider;
pub mod review;
mod sse;
mod types;

use std::sync::{Arc, OnceLock};

pub use error::AiError;
pub use key::ApiKey;
pub use provider::{ChatProvider, EventStream};
pub use tokio_util::sync::CancellationToken;
pub use types::{ChatEvent, ChatMessage, ChatRequest, ChatRole, Usage};

/// The runtime every provider's network work runs on. It's created on first use and lives for the process.
pub fn runtime() -> &'static tokio::runtime::Handle {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .thread_name("looji-ai")
                .enable_all()
                .build()
                .expect("the AI runtime starts on this machine")
        })
        .handle()
}

/// The provider the app uses, built from the stored key. Fails with `NoApiKey` when no key has been saved.
pub fn connect() -> Result<Arc<dyn ChatProvider>, AiError> {
    let key = key_store::load()?.ok_or(AiError::NoApiKey)?;
    Ok(Arc::new(anthropic::AnthropicProvider::new(key)))
}
