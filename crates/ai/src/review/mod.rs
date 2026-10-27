//! Questions asked about a task and the code retrieved for it: whether the task is implemented, and what to watch in
//! review. Each question is fixed, and the answer is read strictly. A reply that doesn't match the expected shape
//! is reported as such, never guessed at.

pub mod check;
pub mod notes;
pub mod testrun;
pub mod verdict;

use looji_domain::Hunk;
use serde_json::Value;

/// The first JSON object in a reply. Models sometimes wrap it in a code fence or add a sentence around it, so the
/// text from the first brace to the last is read. `None` if that isn't a JSON object.
pub(crate) fn json_object(reply: &str) -> Option<Value> {
    let start = reply.find('{')?;
    let end = reply.rfind('}')?;
    let value: Value = serde_json::from_str(&reply[start..=end]).ok()?;
    value.is_object().then_some(value)
}

/// The code shown to the model, each hunk labelled with its file and lines.
pub(crate) fn format_hunks(hunks: &[Hunk]) -> String {
    if hunks.is_empty() {
        return "(no code matched this task)".to_string();
    }
    hunks
        .iter()
        .map(|hunk| {
            format!(
                "### {}:{}-{}\n```\n{}\n```",
                hunk.path, hunk.start_line, hunk.end_line, hunk.text
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

pub(crate) fn task_text(title: &str, description: &str) -> String {
    if description.trim().is_empty() {
        format!("Task: {title}")
    } else {
        format!("Task: {title}\n\n{description}")
    }
}
