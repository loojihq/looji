//! Review notes for a task: what to watch for, what tests are missing, and what to check by hand. Pure data. The
//! notes are made by the AI crate and stored by persistence.

/// What a note is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoteKind {
    Risk,
    MissingTest,
    Check,
}

impl NoteKind {
    pub fn label(self) -> &'static str {
        match self {
            NoteKind::Risk => "risk",
            NoteKind::MissingTest => "missing_test",
            NoteKind::Check => "check",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "risk" => Some(NoteKind::Risk),
            "missing_test" => Some(NoteKind::MissingTest),
            "check" => Some(NoteKind::Check),
            _ => None,
        }
    }
}

/// One note. `files` only ever holds paths that were in the index when the note was made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewNote {
    pub kind: NoteKind,
    pub body: String,
    pub files: Vec<String>,
}
