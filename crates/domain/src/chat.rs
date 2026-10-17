//! The rules of a task chat that don't depend on how replies are sent or drawn: the states a reply moves through, how
//! much history a follow-up carries, and which model a message uses. Pure.

/// Where an assistant reply is. It only moves forward from `Streaming`, and a stopped or failed reply keeps the text
/// it had, so the user can read what arrived.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplyState {
    Streaming,
    Complete,
    /// The user pressed stop. The text so far is kept.
    Stopped,
    /// The connection failed or the provider refused. The text so far is kept, with the reason.
    Failed {
        reason: String,
    },
}

impl ReplyState {
    /// The stored label, and the one read back. Anything else is treated as complete, since only finished replies
    /// are ever stored as anything but streaming.
    pub fn label(&self) -> &'static str {
        match self {
            ReplyState::Streaming | ReplyState::Complete => "complete",
            ReplyState::Stopped => "stopped",
            ReplyState::Failed { .. } => "failed",
        }
    }

    pub fn can_retry(&self) -> bool {
        matches!(self, ReplyState::Failed { .. } | ReplyState::Stopped)
    }
}

/// The reply being shown while it streams. Tokens are appended, and the state changes once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    pub text: String,
    pub state: ReplyState,
}

impl Reply {
    pub fn streaming() -> Self {
        Self {
            text: String::new(),
            state: ReplyState::Streaming,
        }
    }

    /// Adds a token. Ignored once the reply has stopped or failed, so a late token can't change what was kept.
    pub fn push(&mut self, token: &str) {
        if self.state == ReplyState::Streaming {
            self.text.push_str(token);
        }
    }

    pub fn complete(&mut self) {
        if self.state == ReplyState::Streaming {
            self.state = ReplyState::Complete;
        }
    }

    pub fn stop(&mut self) {
        if self.state == ReplyState::Streaming {
            self.state = ReplyState::Stopped;
        }
    }

    pub fn fail(&mut self, reason: impl Into<String>) {
        if self.state == ReplyState::Streaming {
            self.state = ReplyState::Failed {
                reason: reason.into(),
            };
        }
    }
}

/// The most recent messages that fit in a token budget, returned as the index they start at. The newest message is
/// always kept, even when it is over budget on its own, since the user just asked it. Everything older is dropped
/// oldest first.
pub fn recent_within_budget(costs: &[usize], budget: usize) -> usize {
    let mut used = 0;
    let mut start = costs.len();
    for (index, cost) in costs.iter().enumerate().rev() {
        if start < costs.len() && used + cost > budget {
            break;
        }
        used += cost;
        start = index;
    }
    start
}

/// The model a message should use. A saved choice is used if the provider still lists it. Otherwise the default is
/// used, with a notice saying why, so the change isn't silent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelChoice {
    pub model: String,
    pub notice: Option<String>,
}

pub fn resolve_model(saved: Option<&str>, available: &[&str], default: &str) -> ModelChoice {
    match saved {
        None => ModelChoice {
            model: default.to_string(),
            notice: None,
        },
        Some(model) if available.contains(&model) => ModelChoice {
            model: model.to_string(),
            notice: None,
        },
        Some(model) => ModelChoice {
            model: default.to_string(),
            notice: Some(format!(
                "{model} is no longer available. Using {default} instead."
            )),
        },
    }
}

/// A rough token count for prompt budgeting: about four characters to a token.
pub fn approximate_tokens(text: &str) -> usize {
    text.chars().count().div_ceil(4).max(1)
}

#[cfg(test)]
mod tests {
    // Not `use super::*`: gpui's `test` macro would shadow `#[test]`.
    use crate::chat::{Reply, ReplyState, recent_within_budget, resolve_model};

    #[test]
    fn tokens_append_while_streaming_and_a_stop_keeps_what_arrived() {
        let mut reply = Reply::streaming();
        reply.push("Hello");
        reply.push(", world");
        reply.stop();
        reply.push(" late token");

        assert_eq!(reply.text, "Hello, world");
        assert_eq!(reply.state, ReplyState::Stopped);
        assert!(reply.state.can_retry());
    }

    #[test]
    fn a_failure_keeps_the_text_and_the_reason() {
        let mut reply = Reply::streaming();
        reply.push("Partly");
        reply.fail("the connection dropped");
        reply.complete();

        assert_eq!(reply.text, "Partly");
        assert_eq!(
            reply.state,
            ReplyState::Failed {
                reason: "the connection dropped".to_string()
            }
        );
    }

    #[test]
    fn a_finished_reply_is_complete_and_cannot_be_stopped() {
        let mut reply = Reply::streaming();
        reply.push("Done");
        reply.complete();
        reply.stop();

        assert_eq!(reply.state, ReplyState::Complete);
        assert!(!reply.state.can_retry());
    }

    #[test]
    fn history_trims_the_oldest_messages_first_and_keeps_the_newest() {
        // Costs oldest first. With a budget of 10, only the newest two fit.
        let costs = [4, 4, 4, 4];
        assert_eq!(recent_within_budget(&costs, 10), 2);
        assert_eq!(recent_within_budget(&costs, 100), 0);
    }

    #[test]
    fn the_newest_message_is_kept_even_over_budget() {
        assert_eq!(recent_within_budget(&[3, 50], 10), 1);
        assert_eq!(recent_within_budget(&[], 10), 0);
    }

    #[test]
    fn a_removed_model_falls_back_to_the_default_with_a_notice() {
        let available = ["claude-sonnet-5-5", "claude-haiku-5-5"];

        let kept = resolve_model(Some("claude-haiku-5-5"), &available, "claude-sonnet-5-5");
        assert_eq!(kept.model, "claude-haiku-5-5");
        assert_eq!(kept.notice, None);

        let removed = resolve_model(Some("claude-opus-3"), &available, "claude-sonnet-5-5");
        assert_eq!(removed.model, "claude-sonnet-5-5");
        assert!(removed.notice.unwrap().contains("claude-opus-3"));

        let unset = resolve_model(None, &available, "claude-sonnet-5-5");
        assert_eq!(unset.model, "claude-sonnet-5-5");
        assert_eq!(unset.notice, None);
    }
}
