use std::str::FromStr;

use crate::error::DomainError;

/// Where a task is in its life. Columns on the board are presentation on top of this.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Status {
    Backlog,
    Todo,
    InProgress,
    InReview,
    Done,
}

impl Status {
    /// Every status, least to most advanced. "Nearest visible" fallbacks are measured along
    /// this order, so changing it changes where hidden tasks land.
    pub const ALL: [Status; 5] = [
        Status::Backlog,
        Status::Todo,
        Status::InProgress,
        Status::InReview,
        Status::Done,
    ];

    /// The four statuses the board always shows. A project can't hide these.
    pub const DEFAULTS: [Status; 4] = [
        Status::Backlog,
        Status::Todo,
        Status::InProgress,
        Status::Done,
    ];

    /// Whether this is one of the four statuses that can never be hidden.
    pub fn is_default(self) -> bool {
        Self::DEFAULTS.contains(&self)
    }

    /// The key written to the database. These strings are stored data:
    /// renaming a variant must not change them.
    pub fn as_str(&self) -> &'static str {
        match self {
            Status::Backlog => "backlog",
            Status::Todo => "todo",
            Status::InProgress => "in_progress",
            Status::InReview => "in_review",
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_are_the_four_that_are_never_hidden() {
        assert_eq!(Status::DEFAULTS.len(), 4);
        assert!(Status::DEFAULTS.iter().all(|status| status.is_default()));
        assert!(!Status::InReview.is_default());
    }

    #[test]
    fn every_status_is_a_default_or_the_one_optional_status() {
        let optional: Vec<Status> = Status::ALL
            .into_iter()
            .filter(|status| !status.is_default())
            .collect();
        assert_eq!(optional, vec![Status::InReview]);
    }

    #[test]
    fn stored_keys_round_trip() {
        for status in Status::ALL {
            assert_eq!(status.as_str().parse::<Status>(), Ok(status));
        }
    }
}
