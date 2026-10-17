//! The provider trait, and the stream every provider returns.
//!
//! Cancellation is handled here, once, for every provider. A stream is cancelled when the caller's token is
//! cancelled, or when the stream is dropped. Either way the producing task sees the cancel and stops, and the
//! stream yields `AiError::Cancelled` once and then ends. Tokens that arrived before the cancel are still
//! delivered, so the caller keeps what it had.

use std::{
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use futures_util::{Stream, future::BoxFuture};
use tokio::sync::mpsc;
use tokio_util::sync::{CancellationToken, DropGuard, WaitForCancellationFutureOwned};

use crate::{
    error::AiError,
    types::{ChatEvent, ChatRequest},
};

/// Events from one response, in order.
pub type EventStream = Pin<Box<dyn Stream<Item = Result<ChatEvent, AiError>> + Send>>;

/// A chat provider. Returns a boxed future, so it can be used as `dyn ChatProvider` when the provider is only
/// chosen at runtime.
///
/// Call it from a Tokio runtime. Providers spawn tasks for the network work, and those tasks need one.
pub trait ChatProvider: Send + Sync {
    /// Starts a response. The future resolves once the provider has accepted the request, which is when a
    /// failure like a bad key or a rate limit shows up. Tokens arrive on the returned stream.
    fn stream_chat(
        &self,
        request: ChatRequest,
        cancel: CancellationToken,
    ) -> BoxFuture<'_, Result<EventStream, AiError>>;
}

/// Events waiting between the producing task and the stream. Small, so a slow reader slows the producer.
const STREAM_BUFFER: usize = 16;

/// The producing side of a stream, held by the task that makes the events.
pub struct Producer {
    tx: mpsc::Sender<Result<ChatEvent, AiError>>,
    cancel: CancellationToken,
}

impl Producer {
    /// Sends one event. Returns `false` once the stream has been cancelled or dropped, and the producer should
    /// stop. Cancel is checked first, so a send doesn't win over a cancel that's already happened.
    pub async fn send(&self, item: Result<ChatEvent, AiError>) -> bool {
        tokio::select! {
            biased;
            _ = self.cancel.cancelled() => false,
            sent = self.tx.send(item) => sent.is_ok(),
        }
    }

    /// Waits for `duration`. Returns `false` if the stream stopped during the wait.
    pub async fn pause(&self, duration: Duration) -> bool {
        tokio::select! {
            biased;
            _ = self.cancel.cancelled() => false,
            _ = tokio::time::sleep(duration) => true,
        }
    }

    /// Resolves once the stream has stopped, for a producer that is waiting on something else.
    pub async fn stopped(&self) {
        self.cancel.cancelled().await
    }
}

/// The reading side of a stream. See the module docs for when it ends.
pub struct TokenStream {
    rx: mpsc::Receiver<Result<ChatEvent, AiError>>,
    run: CancellationToken,
    cancelled: Pin<Box<WaitForCancellationFutureOwned>>,
    /// Cancels the producer when the stream is dropped, so forgetting to cancel is safe.
    _stop_on_drop: DropGuard,
    done: bool,
}

impl Stream for TokenStream {
    type Item = Result<ChatEvent, AiError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        if this.done {
            return Poll::Ready(None);
        }
        // Checked before the channel, so a cancel stops delivery even when events are already queued. The
        // waker is registered too, so a cancel while the reader is waiting wakes it.
        if this.cancelled.as_mut().poll(cx).is_ready() || this.run.is_cancelled() {
            this.done = true;
            this.rx.close();
            return Poll::Ready(Some(Err(AiError::Cancelled)));
        }
        match this.rx.poll_recv(cx) {
            Poll::Ready(Some(item)) => Poll::Ready(Some(item)),
            Poll::Ready(None) => {
                this.done = true;
                Poll::Ready(None)
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

/// Opens a stream whose producer is cancelled by `caller`, and by dropping the stream.
pub fn open_stream(caller: &CancellationToken) -> (Producer, TokenStream) {
    // A child token, so dropping the stream cancels only this stream and never the caller's token.
    let run = caller.child_token();
    let (tx, rx) = mpsc::channel(STREAM_BUFFER);
    let stream = TokenStream {
        rx,
        run: run.clone(),
        cancelled: Box::pin(run.clone().cancelled_owned()),
        _stop_on_drop: run.clone().drop_guard(),
        done: false,
    };
    (Producer { tx, cancel: run }, stream)
}
