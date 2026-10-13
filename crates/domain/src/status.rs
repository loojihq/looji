use std::str::FromStr;

use crate::error::DomainError;

/// Where a task is in its life. Columns on the board are presentation on top of this.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Status {
    Backlog,
    Todo,
    InProgress,
    Done,
}

impl Status {
    pub const ALL: [Status; 4] = [
        Status::Backlog,
        Status::Todo,
        Status::InProgress,
        Status::Done,
    ];

    /// The key written to the database. These strings are stored data:
    /// renaming a variant must not change them.
    pub fn as_str(&self) -> &'static str {
        match self {
            Status::Backlog => "backlog",
            Status::Todo => "todo",
            Status::InProgress => "in_progress",
            Status::Done => "done",
        }
    }
}

impl FromStr for Status {
    type Err = DomainError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Status::ALL
            .into_iter()
            .find(|status| status.as_str() == text)
            .ok_or_else(|| DomainError::UnknownStatus(text.to_string()))
    }
}
