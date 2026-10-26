//! The request for a message in a task chat: the task's details, the code that matches it (when the user has
//! repository awareness on), and as much of the conversation as fits a budget. The oldest messages go first, so the
//! newest question and the replies it follows from are the ones the model sees.

use looji_domain::{Hunk, approximate_tokens, recent_within_budget};

use crate::review::format_hunks;
use crate::{ChatMessage, ChatRequest};

/// Tokens of conversation sent with each message, at most.
pub const HISTORY_BUDGET_TOKENS: usize = 6_000;
/// The longest reply asked for.
pub const MAX_REPLY_TOKENS: u32 = 2_000;

/// The task a conversation is about, as it is now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaskBrief<'a> {
    pub title: &'a str,
    pub description: &'a str,
    pub status: &'a str,
    /// The conversation is about the whole project and its codebase, and `title` is the project's name.
    pub whole_project: bool,
}

/// The request for the conversation's next reply. `history` is every message so far, oldest first, and it ends with
/// the user's new message. Returns the request, and how many of the messages were left out.
pub fn build_chat_request(
    model: &str,
    brief: TaskBrief<'_>,
    history: &[ChatMessage],
    hunks: &[Hunk],
    budget_tokens: usize,
) -> (ChatRequest, usize) {
    let costs: Vec<usize> = history
        .iter()
        .map(|message| approximate_tokens(&message.content))
        .collect();
    let start = recent_within_budget(&costs, budget_tokens);
    let kept = history[start..].to_vec();

    let mut system = if brief.whole_project {
        format!(
            "You are helping with a software project, using its repository. Answer about the whole project and the \
             code shown, and say so when the code shown is not enough to answer. Earlier messages in this \
             conversation are included, and a follow-up may refer to them.\n\nProject: {}",
            brief.title
        )
    } else {
        format!(
            "You are helping with one task in a project plan. Answer about this task and the code shown. Earlier messages \
             in this conversation are included, and a follow-up may refer to them.\n\nTask: {}\nStatus: {}",
            brief.title, brief.status
        )
    };
    if !brief.description.trim().is_empty() {
        system.push_str("\n\nDescription:\n");
        system.push_str(brief.description.trim());
    }
    if !hunks.is_empty() {
        system.push_str("\n\nRelevant code from the repository:\n\n");
        system.push_str(&format_hunks(hunks));
    }

    (
        ChatRequest {
            model: model.to_string(),
            system: Some(system),
            messages: kept,
            max_tokens: MAX_REPLY_TOKENS,
        },
        start,
    )
}

/// The request for a reply from an agent, which looks the code up itself instead of being handed it. The instructions
/// say how to investigate and how to cite what it found.
pub fn build_agent_request(
    model: &str,
    brief: TaskBrief<'_>,
    history: &[ChatMessage],
    budget_tokens: usize,
) -> (ChatRequest, usize) {
    let (mut request, left_out) = build_chat_request(model, brief, history, &[], budget_tokens);
    let about = if brief.whole_project {
        "the project and its repository"
    } else {
        "one task in the project and its repository"
    };
    let mut system = format!(
        "You answer questions about {about}. You have tools that search and read the project's code, docs, issues and \
         commits. Look before you answer: search with words from the question and likely names, read the code around \
         what you find, and follow it until you can answer. Do not guess about code you have not read.\n\n\
         Write only the answer. Do not narrate what you are about to look up. Name files as `path:line` where you \
         rely on them, so the reader can check. If you could not find something, say what you looked for and where.\n\n"
    );
    if let Some(original) = request.system.take() {
        system.push_str(&original);
    }
    request.system = Some(system);
    (request, left_out)
}
