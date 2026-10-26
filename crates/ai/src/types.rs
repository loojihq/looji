//! The plain request and response types the rest of the app uses. Nothing here names a vendor.

/// Who wrote a message. The system prompt is a separate field on the request, not a role.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatRole {
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatMessage {
    pub role: ChatRole,
    pub content: String,
}

impl ChatMessage {
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: ChatRole::User,
            content: content.into(),
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: ChatRole::Assistant,
            content: content.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatRequest {
    pub model: String,
    pub system: Option<String>,
    pub messages: Vec<ChatMessage>,
    pub max_tokens: u32,
}

/// Token counts the provider reported for a whole response.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

/// Something an answer drew on, so it can be cited: a file and lines, an issue, or a commit.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Source {
    /// `code`, `doc`, `issue`, or `commit`.
    pub kind: String,
    /// The file path, issue id, or commit id.
    pub reference: String,
    /// The symbol, issue title, or commit subject.
    pub label: String,
    /// The lines of a file. Zero for issues and commits.
    pub start_line: u32,
    pub end_line: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatEvent {
    /// Text as it arrived. Tokens are not necessarily words.
    Token(String),
    /// What an agent is doing between pieces of text, in a few words: "Searching for password checks".
    Step(String),
    /// Something the agent read, which the answer can cite.
    Source(Source),
    /// The response is complete. `usage` is `None` when the provider didn't report it.
    Finished { usage: Option<Usage> },
}
