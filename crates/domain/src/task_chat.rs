//! A conversation about a task: who said what, with which model, and when. Pure data.

use crate::Timestamp;

/// Who wrote a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageRole {
    User,
    Assistant,
}

impl MessageRole {
    pub fn label(self) -> &'static str {
        match self {
            MessageRole::User => "user",
            MessageRole::Assistant => "assistant",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "user" => Some(MessageRole::User),
            "assistant" => Some(MessageRole::Assistant),
            _ => None,
        }
    }
}

/// One message in a thread. `model` is the model that wrote an assistant message, and `None` for the user's own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatMessage {
    pub role: MessageRole,
    pub content: String,
    pub model: Option<String>,
    pub created_at: Timestamp,
}
