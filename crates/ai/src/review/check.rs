//! Whether a task is implemented. The model is shown the task and the retrieved code, and asked for one verdict and
//! the files that show it. Only a reply that reads cleanly counts. Anything else is `Unknown`, and `Implemented` is
//! never given without a file to point to.

use std::collections::BTreeSet;

use looji_domain::Hunk;

use super::{format_hunks, json_object, task_text};
use crate::{
    AiError, CancellationToken, ChatMessage, ChatProvider, ChatRequest, generation::complete_with,
};

/// The fixed question. The reply is one JSON object, so the parser has one shape to read.
pub const CHECK_QUESTION: &str = "Is the task described below implemented in the code shown? Reply with only a JSON \
object of this shape: {\"verdict\": \"implemented\" | \"partial\" | \"not_started\", \"evidence\": [\"path of a file \
that shows it\"]}. Say \"implemented\" only when the code shown does the whole task. Say \"partial\" when some of it \
is there. Say \"not_started\" when none of it is. List only paths that appear in the code shown.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Implemented,
    Partial,
    NotStarted,
    /// The reply couldn't be read, or it didn't name a file for a verdict that needs one.
    Unknown,
}

impl Verdict {
    pub fn label(self) -> &'static str {
        match self {
            Verdict::Implemented => "implemented",
            Verdict::Partial => "partial",
            Verdict::NotStarted => "not started",
            Verdict::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskCheck {
    pub verdict: Verdict,
    /// Files that were shown to the model and that the model named. Never a file that wasn't shown.
    pub evidence: Vec<String>,
}

impl TaskCheck {
    fn unknown() -> Self {
        Self {
            verdict: Verdict::Unknown,
            evidence: Vec::new(),
        }
    }
}

pub fn build_check_request(
    model: &str,
    title: &str,
    description: &str,
    hunks: &[Hunk],
) -> ChatRequest {
    ChatRequest {
        model: model.to_string(),
        system: Some(CHECK_QUESTION.to_string()),
        messages: vec![ChatMessage::user(format!(
            "{}\n\nCode:\n\n{}",
            task_text(title, description),
            format_hunks(hunks)
        ))],
        max_tokens: 400,
    }
}

/// Reads a reply. `shown` is the set of files that were shown to the model.
pub fn parse_verdict(reply: &str, shown: &BTreeSet<&str>) -> TaskCheck {
    let Some(value) = json_object(reply) else {
        return TaskCheck::unknown();
    };
    let Some(word) = value.get("verdict").and_then(|verdict| verdict.as_str()) else {
        return TaskCheck::unknown();
    };
    let verdict = match word
        .trim()
        .to_ascii_lowercase()
        .replace([' ', '-'], "_")
        .as_str()
    {
        "implemented" => Verdict::Implemented,
        "partial" => Verdict::Partial,
        "not_started" => Verdict::NotStarted,
        _ => return TaskCheck::unknown(),
    };
    let mut evidence: Vec<String> = Vec::new();
    for item in value
        .get("evidence")
        .and_then(|evidence| evidence.as_array())
        .into_iter()
        .flatten()
    {
        if let Some(path) = item.as_str()
            && shown.contains(path)
            && !evidence.iter().any(|kept| kept == path)
        {
            evidence.push(path.to_string());
        }
    }
    if verdict == Verdict::Implemented && evidence.is_empty() {
        return TaskCheck::unknown();
    }
    TaskCheck { verdict, evidence }
}

/// Asks the model and reads the answer. Fails only when no reply arrives at all.
pub async fn check_task(
    provider: &dyn ChatProvider,
    model: &str,
    title: &str,
    description: &str,
    hunks: &[Hunk],
    cancel: CancellationToken,
) -> Result<TaskCheck, AiError> {
    let request = build_check_request(model, title, description, hunks);
    let reply = complete_with(provider, request, cancel).await?;
    let shown: BTreeSet<&str> = hunks.iter().map(|hunk| hunk.path.as_str()).collect();
    Ok(parse_verdict(&reply, &shown))
}
