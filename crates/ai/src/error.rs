//! The errors a chat request can end with. The UI matches on these to tell the cases apart.

use std::time::Duration;

use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AiError {
    /// No key is stored, so no request was sent.
    #[error("no API key is stored")]
    NoApiKey,
    /// The provider refused the key (401 or 403).
    #[error("the provider rejected the API key")]
    AuthFailed,
    /// The provider is limiting requests (429). `retry_after` is the provider's own hint, when it sent one.
    /// Nothing retries on its own, so the caller decides whether and when to try again.
    #[error("the provider is rate limiting requests")]
    RateLimited { retry_after: Option<Duration> },
    /// The provider is overloaded or failing (5xx, or 529). This is usually temporary.
    #[error("the provider is unavailable (status {status})")]
    Unavailable { status: u16 },
    /// The provider refused the request for a reason other than the key or the rate.
    #[error("the provider refused the request (status {status})")]
    Rejected { status: u16 },
    /// The connection failed or dropped. The message carries no request content.
    #[error("network error: {0}")]
    Network(String),
    /// The provider sent something this client can't read.
    #[error("unexpected response from the provider: {0}")]
    Protocol(String),
    /// Claude Code couldn't be run, or it stopped without an answer. The message is written for the user.
    #[error("{0}")]
    CommandFailed(String),
    /// The caller stopped the stream. Text that arrived before this is still valid.
    #[error("the request was cancelled")]
    Cancelled,
    /// The key store couldn't be read or written.
    #[error("the key store could not be used: {0}")]
    KeyStore(String),
}
