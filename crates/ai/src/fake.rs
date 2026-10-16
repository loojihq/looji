//! A scripted provider for tests, including the tests of the UI. It plays back one script per request, in order,
//! and records every request it receives.
//!
//! Each script is a list of steps. A token is delivered as an event, a pause waits, a failure ends the stream
//! with an error, and an end finishes it. A script can also refuse the request itself, so `stream_chat` fails
//! before a stream exists, as a bad key does.

use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::Duration,
};

use futures_util::future::BoxFuture;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use crate::{
    error::AiError,
    provider::{ChatProvider, EventStream, Producer, open_stream},
    types::{ChatEvent, ChatRequest, Usage},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    Token(String),
    Pause(Duration),
    /// Ends the stream with this error.
    Fail(AiError),
    /// Finishes the stream normally.
    End(Option<Usage>),
}

/// One response's worth of steps, or a refusal of the request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Script {
    refusal: Option<AiError>,
    steps: Vec<Step>,
}

impl Script {
    pub fn new(steps: Vec<Step>) -> Self {
        Self {
            refusal: None,
            steps,
        }
    }

    /// A script whose request fails before any stream exists.
    pub fn refused(error: AiError) -> Self {
        Self {
            refusal: Some(error),
            steps: Vec::new(),
        }
    }

    /// Each word as a token, then a normal finish.
    pub fn tokens<I, S>(words: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut steps: Vec<Step> = words
            .into_iter()
            .map(|word| Step::Token(word.into()))
            .collect();
        steps.push(Step::End(None));
        Self::new(steps)
    }

    /// `count` tokens, each followed by a pause. Never finishes, so the caller has to cancel.
    pub fn ticking(count: usize, every: Duration) -> Self {
        let steps = (0..count)
            .flat_map(|index| [Step::Token(format!("t{index} ")), Step::Pause(every)])
            .collect();
        Self::new(steps)
    }
}

/// How many producer tasks have stopped. A test waits on it to prove the producer really ended, rather than
/// trusting that it would.
#[derive(Debug)]
pub struct StopCount {
    tx: watch::Sender<usize>,
}

impl StopCount {
    fn new() -> Self {
        Self {
            tx: watch::channel(0).0,
        }
    }

    pub fn get(&self) -> usize {
        *self.tx.borrow()
    }

    /// Waits until at least `count` producers have stopped. Returns `false` if that takes longer than `within`.
    pub async fn wait_until(&self, count: usize, within: Duration) -> bool {
        let mut rx = self.tx.subscribe();
        matches!(
            tokio::time::timeout(within, rx.wait_for(|stopped| *stopped >= count)).await,
            Ok(Ok(_))
        )
    }

    fn record(&self) {
        self.tx.send_modify(|stopped| *stopped += 1);
    }
}

/// Records a stop when the producer task ends, whichever way it ends.
struct StopOnDrop(Arc<StopCount>);

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        self.0.record();
    }
}

#[derive(Default)]
pub struct FakeProvider {
    scripts: Mutex<VecDeque<Script>>,
    requests: Mutex<Vec<ChatRequest>>,
    stops: Arc<StopCount>,
}

impl Default for StopCount {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeProvider {
    pub fn new() -> Self {
        Self::default()
    }

    /// Queues the script for a future request. Scripts are used in the order they were queued.
    pub fn push(&self, script: Script) {
        self.scripts
            .lock()
            .expect("the script queue isn't poisoned")
            .push_back(script);
    }

    /// Every request received so far, in order.
    pub fn requests(&self) -> Vec<ChatRequest> {
        self.requests
            .lock()
            .expect("the request log isn't poisoned")
            .clone()
    }

    /// The count of stopped producer tasks.
    pub fn stops(&self) -> Arc<StopCount> {
        Arc::clone(&self.stops)
    }
}

impl ChatProvider for FakeProvider {
    fn stream_chat(
        &self,
        request: ChatRequest,
        cancel: CancellationToken,
    ) -> BoxFuture<'_, Result<EventStream, AiError>> {
        Box::pin(async move {
            self.requests
                .lock()
                .expect("the request log isn't poisoned")
                .push(request);
            let script = self
                .scripts
                .lock()
                .expect("the script queue isn't poisoned")
                .pop_front()
                .ok_or_else(|| {
                    AiError::Protocol("the fake provider has no scripted response left".to_string())
                })?;
            if let Some(error) = script.refusal {
                return Err(error);
            }
            let (producer, stream) = open_stream(&cancel);
            let stops = Arc::clone(&self.stops);
            tokio::spawn(async move {
                let _stop = StopOnDrop(stops);
                play(script.steps, producer).await;
            });
            Ok(Box::pin(stream) as EventStream)
        })
    }
}

/// Plays the steps out, stopping as soon as the stream is cancelled or dropped.
async fn play(steps: Vec<Step>, out: Producer) {
    for step in steps {
        let keep_going = match step {
            Step::Token(text) => out.send(Ok(ChatEvent::Token(text))).await,
            Step::Pause(duration) => out.pause(duration).await,
            Step::Fail(error) => {
                out.send(Err(error)).await;
                false
            }
            Step::End(usage) => {
                out.send(Ok(ChatEvent::Finished { usage })).await;
                false
            }
        };
        if !keep_going {
            return;
        }
    }
}
