//! A provider-agnostic chat client. The rest of the app calls [`ChatProvider`] and never sees a vendor's types.
//!
//! The network work runs on a Tokio runtime that this crate owns (see [`runtime`]), so it never blocks the UI's
//! thread. Events come back through the stream, which the caller reads from wherever it likes.
//!
//! Nothing here depends on GPUI or SQLite. Only the UI should depend on this crate.

pub mod anthropic;
pub mod chat;
pub mod claude_code;
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

/// Which provider answers. Claude Code runs the local `claude` command with the user's own sign-in. The Anthropic
/// option calls the API with a key saved in the system's key store.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    ClaudeCode,
    Anthropic,
}

impl Provider {
    /// Every choice, in the order the settings offer them.
    pub const ALL: [Provider; 2] = [Provider::ClaudeCode, Provider::Anthropic];

    /// The name the setting is stored under.
    pub fn stored(self) -> &'static str {
        match self {
            Provider::ClaudeCode => "claude_code",
            Provider::Anthropic => "anthropic",
        }
    }

    /// The choice a stored name stands for. Unknown names give `None`.
    pub fn from_stored(text: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|provider| provider.stored() == text)
    }

    /// Whether the provider signs in with an API key the app stores. The others use their own sign-in.
    pub fn needs_key(self) -> bool {
        matches!(self, Provider::Anthropic)
    }

    /// The name shown in the settings.
    pub fn label(self) -> &'static str {
        match self {
            Provider::ClaudeCode => "Claude Code (local)",
            Provider::Anthropic => "Anthropic API",
        }
    }
}

/// The provider for a choice. Claude Code fails with a message when its command isn't installed, and the API
/// fails with `NoApiKey` when no key has been saved.
pub fn connect(provider: Provider) -> Result<Arc<dyn ChatProvider>, AiError> {
    match provider {
        Provider::ClaudeCode => {
            if claude_code::installed() {
                Ok(Arc::new(claude_code::ClaudeCodeProvider::new()))
            } else {
                Err(AiError::CommandFailed(
                    "Claude Code isn't installed on this computer. Install it and sign in, then try again."
                        .to_string(),
                ))
            }
        }
        Provider::Anthropic => {
            let key = key_store::load()?.ok_or(AiError::NoApiKey)?;
            Ok(Arc::new(anthropic::AnthropicProvider::new(key)))
        }
    }
}
