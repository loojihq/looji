//! Review notes for a task: risks, missing tests, and checks to do by hand. Every file a note names is checked against
//! the repository's index, and a name that isn't in it is dropped. A repository with no test files always gets a note
//! saying so, whatever the model wrote.

use std::collections::BTreeSet;

use looji_domain::{NoteKind, ReviewNote, is_test_path};

use super::{format_hunks, json_object, task_text};
use crate::{
    AiError, CancellationToken, ChatMessage, ChatProvider, ChatRequest, generation::complete_with,
};
use looji_domain::Hunk;

pub const NOTES_QUESTION: &str = "Write review notes for the task described below, from the code shown. Reply with \
only a JSON object of this shape: {\"notes\": [{\"kind\": \"risk\" | \"missing_test\" | \"check\", \"text\": \"one or \
two sentences\", \"files\": [\"paths that appear in the code shown\"]}]}. A risk is something in this change that could \
break. A missing_test names behaviour that no test covers. A check is something a person should confirm by hand. Use \
only paths that appear in the code shown.";

/// The note added whenever the repository has no test files, whatever the model said.
pub const NO_TESTS_NOTE: &str = "This repository has no test files, so nothing checks that this task's changes keep \
working. Add tests for the behaviour the task changes.";

/// What went wrong with a reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotesError {
    /// The reply had no JSON object with a `notes` list.
    Unreadable,
}

pub fn build_notes_request(
    model: &str,
    title: &str,
    description: &str,
    hunks: &[Hunk],
) -> ChatRequest {
    ChatRequest {
        model: model.to_string(),
        system: Some(NOTES_QUESTION.to_string()),
        messages: vec![ChatMessage::user(format!(
            "{}\n\nCode:\n\n{}",
            task_text(title, description),
            format_hunks(hunks)
        ))],
        max_tokens: 1_200,
    }
}

/// Reads a reply. Notes with an unknown kind or no text are left out. File names that aren't in the index are
/// removed from the notes that name them, and the notes themselves are kept.
pub fn parse_notes(
    reply: &str,
    index_files: &BTreeSet<&str>,
) -> Result<Vec<ReviewNote>, NotesError> {
    let value = json_object(reply).ok_or(NotesError::Unreadable)?;
    let items = value
        .get("notes")
        .and_then(|notes| notes.as_array())
        .ok_or(NotesError::Unreadable)?;

    let mut notes = Vec::new();
    for item in items {
        let kind = item
            .get("kind")
            .and_then(|kind| kind.as_str())
            .and_then(|kind| NoteKind::parse(kind.trim()));
        let text = item
            .get("text")
            .and_then(|text| text.as_str())
            .map(str::trim)
            .filter(|text| !text.is_empty());
        let (Some(kind), Some(text)) = (kind, text) else {
            continue;
        };
        let mut files: Vec<String> = Vec::new();
        for file in item
            .get("files")
            .and_then(|files| files.as_array())
            .into_iter()
            .flatten()
            .filter_map(|file| file.as_str())
        {
            if index_files.contains(file) && !files.iter().any(|kept| kept == file) {
                files.push(file.to_string());
            }
        }
        notes.push(ReviewNote {
            kind,
            body: text.to_string(),
            files,
        });
    }
    Ok(notes)
}

/// Adds the no-tests note when the index has no test file. Nothing is added when there are tests, so the note
/// appears exactly when it's true.
pub fn add_test_gap_note(notes: &mut Vec<ReviewNote>, index_files: &[String]) {
    if index_files.iter().any(|path| is_test_path(path)) {
        return;
    }
    notes.push(ReviewNote {
        kind: NoteKind::MissingTest,
        body: NO_TESTS_NOTE.to_string(),
        files: Vec::new(),
    });
}

/// Asks the model for notes, then checks them against the index and adds the no-tests note when it applies.
pub async fn review_task(
    provider: &dyn ChatProvider,
    model: &str,
    title: &str,
    description: &str,
    hunks: &[Hunk],
    index_files: &[String],
    cancel: CancellationToken,
) -> Result<Vec<ReviewNote>, AiError> {
    let request = build_notes_request(model, title, description, hunks);
    let reply = complete_with(provider, request, cancel).await?;
    let known: BTreeSet<&str> = index_files.iter().map(String::as_str).collect();
    let mut notes = parse_notes(&reply, &known)
        .map_err(|_| AiError::Protocol("the review notes reply had no notes list".to_string()))?;
    add_test_gap_note(&mut notes, index_files);
    Ok(notes)
}
