//! Editing a task's priority and tags from the card or the panel. Both surfaces write through
//! `edit_task`, the one write path, so a change shows up on the card and in the panel alike. This file
//! also holds the tag editor's state: the draft being typed, the suggestions, and the keys.
//!
//! Tags are stored normalised, so what's typed is normalised by the domain before it's written. The
//! suggestions come from the tags tasks in the project have right now, so a tag that no task uses any
//! more is never offered.

use gpui::{prelude::*, *};
use looji_domain::{
    DomainError, Priority, Task, TaskEventKind, TaskId, TaskRepository, changed_fields,
    normalize_tag,
};

use super::{WorkspaceView, store, typed_text};

/// How many suggestions are offered at most.
pub(super) const MAX_SUGGESTIONS: usize = 8;

/// The tag being typed for one task. Its existence is what routes keys to the tag input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TagEdit {
    pub task: TaskId,
    /// Text typed but not yet a tag.
    pub draft: String,
    /// Tags in use in the project that match the draft and that the task doesn't have.
    pub suggestions: Vec<String>,
    /// The suggestion Enter would choose, if one is highlighted.
    pub highlight: Option<usize>,
    /// Why the last tag wasn't added.
    pub error: Option<String>,
}

impl TagEdit {
    fn new(task: TaskId) -> Self {
        Self {
            task,
            draft: String::new(),
            suggestions: Vec::new(),
            highlight: None,
            error: None,
        }
    }
}

/// The suggestions to offer: the tags in use, without the ones the task already has. Order is kept.
pub(super) fn offered(in_use: Vec<String>, on_task: &[String]) -> Vec<String> {
    in_use
        .into_iter()
        .filter(|tag| !on_task.contains(tag))
        .collect()
}

/// Moves the highlight one step down or up through `len` suggestions, stopping at the ends. Stepping
/// up from the first suggestion clears the highlight, so Enter adds what was typed again.
pub(super) fn move_highlight(current: Option<usize>, len: usize, down: bool) -> Option<usize> {
    if len == 0 {
        return None;
    }
    match (current, down) {
        (None, true) => Some(0),
        (None, false) => Some(len - 1),
        (Some(index), true) => Some((index + 1).min(len - 1)),
        (Some(0), false) => None,
        (Some(index), false) => Some(index - 1),
    }
}

impl WorkspaceView {
    /// Writes an edit to a task: applies it, records the changed fields in the history, saves both in
    /// one transaction, then updates the board's copy in place. Nothing is re-read for the board, so a
    /// card changes at once. An open panel for this task shows the new history too.
    ///
    /// Returns the message to show when the edit wasn't saved. An edit that changes nothing is a
    /// success and writes nothing.
    pub(super) fn edit_task(
        &mut self,
        task: TaskId,
        apply: impl FnOnce(&mut Task) -> Result<(), DomainError>,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let Some(index) = self.board.iter().position(|other| other.id() == task) else {
            return Err("That task is no longer on the board.".to_string());
        };
        let before = self.board[index].clone();
        let mut after = before.clone();
        apply(&mut after).map_err(|err| err.to_string())?;
        let fields = changed_fields(&before, &after);
        if fields.is_empty() {
            return Ok(());
        }
        let event = TaskEventKind::Edited {
            fields: fields.into_iter().map(str::to_string).collect(),
        };
        if let Err(err) = store(cx).save_task_recording(&after, &event) {
            eprintln!("could not save task {}: {err}", task.as_uuid());
            return Err("Could not save that change. Try again.".to_string());
        }
        super::panel::replace_task(&mut self.board, after);
        if let Some(panel) = self.panel.as_mut().filter(|panel| panel.task == task) {
            let history = store(cx).task_history(task).unwrap_or_default();
            panel.history = history.into_iter().rev().collect();
            panel.error = None;
        }
        cx.notify();
        Ok(())
    }

    /// Sets the priority of a task, from the card or the panel.
    pub(super) fn write_task_priority(
        &mut self,
        task: TaskId,
        priority: Option<Priority>,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        self.edit_task(
            task,
            |task| {
                task.set_priority(priority);
                Ok(())
            },
            cx,
        )
    }

    /// Opens the tag input for a task. Keys go to it until it's blurred. Any draft for another task is
    /// added first.
    pub(super) fn focus_tag_input(
        &mut self,
        task: TaskId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.tag_edit.as_ref().is_some_and(|edit| edit.task == task) {
            return;
        }
        self.blur_tag_editor(cx);
        self.tag_edit = Some(TagEdit::new(task));
        self.focus_handle.focus(window);
        cx.notify();
    }

    /// Ends typing. A draft that's still there is added first, so a click elsewhere doesn't lose it.
    /// If it can't be added, the reason is logged. The draft is gone by then, so it can't be shown.
    pub(super) fn blur_tag_editor(&mut self, cx: &mut Context<Self>) {
        let Some(edit) = self.tag_edit.take() else {
            return;
        };
        if !edit.draft.trim().is_empty()
            && let Err(message) = self.add_tag_text(edit.task, &edit.draft, cx)
        {
            eprintln!("a typed tag was not added: {message}");
        }
        cx.notify();
    }

    /// Opens a card for tag and priority editing, and puts the keyboard on its tag input.
    pub(super) fn open_card_editor(
        &mut self,
        task: TaskId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.card_editing = Some(task);
        self.focus_tag_input(task, window, cx);
    }

    /// Closes a card's editor. Typed text is added first, as for any blur.
    pub(super) fn close_card_editor(&mut self, cx: &mut Context<Self>) {
        self.blur_tag_editor(cx);
        self.card_editing = None;
        cx.notify();
    }

    /// Types text into the tag input. A comma ends the tag before it, as in the dialog.
    pub(super) fn type_tag_text(&mut self, text: &str, cx: &mut Context<Self>) {
        let mut parts = text.split(',').peekable();
        while let Some(part) = parts.next() {
            if let Some(edit) = self.tag_edit.as_mut() {
                edit.draft.push_str(part);
                edit.highlight = None;
                edit.error = None;
            }
            if parts.peek().is_some() {
                self.commit_tag(cx);
            }
        }
        self.refresh_suggestions(cx);
        cx.notify();
    }

    /// Adds the highlighted suggestion if there is one, otherwise the draft. The draft is cleared on
    /// success. A failure stays in the input with the reason shown.
    pub(super) fn commit_tag(&mut self, cx: &mut Context<Self>) {
        let Some(edit) = self.tag_edit.as_ref() else {
            return;
        };
        let task = edit.task;
        let tag = edit
            .highlight
            .and_then(|index| edit.suggestions.get(index).cloned())
            .unwrap_or_else(|| edit.draft.clone());
        if tag.trim().is_empty() {
            return cx.notify();
        }
        match self.add_tag_text(task, &tag, cx) {
            Ok(()) => {
                if let Some(edit) = self.tag_edit.as_mut() {
                    edit.draft.clear();
                    edit.highlight = None;
                    edit.suggestions.clear();
                    edit.error = None;
                }
            }
            Err(message) => self.set_tag_error(message, cx),
        }
        cx.notify();
    }

    /// Clicking a suggestion adds it straight away.
    pub(super) fn choose_suggestion(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(edit) = self.tag_edit.as_mut() {
            edit.highlight = Some(index);
        }
        self.commit_tag(cx);
    }

    /// Removes a tag from a task, from its chip.
    pub(super) fn remove_task_tag(&mut self, task: TaskId, tag: String, cx: &mut Context<Self>) {
        if let Err(message) = self.edit_task(
            task,
            |task| {
                task.remove_tag(&tag);
                Ok(())
            },
            cx,
        ) {
            self.set_tag_error(message, cx);
        }
        self.refresh_suggestions(cx);
    }

    /// A key while the tag input is focused. Returns whether the input used it, so the rest of the app
    /// doesn't also act on it.
    pub(super) fn tag_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) -> bool {
        let Some(edit) = self.tag_edit.as_ref() else {
            return false;
        };
        let task = edit.task;
        let showing = !edit.suggestions.is_empty();
        let modifiers = event.keystroke.modifiers;
        match event.keystroke.key.as_str() {
            // Escape closes the suggestions first, then the input, then the card's editor.
            "escape" if showing => {
                if let Some(edit) = self.tag_edit.as_mut() {
                    edit.suggestions.clear();
                    edit.highlight = None;
                }
            }
            "escape" => {
                self.blur_tag_editor(cx);
                self.card_editing = None;
            }
            "enter" => self.commit_tag(cx),
            "down" if showing => self.step_highlight(true),
            "up" if showing => self.step_highlight(false),
            "backspace" => {
                let draft_empty = edit.draft.is_empty();
                if draft_empty {
                    let last = self
                        .board
                        .iter()
                        .find(|other| other.id() == task)
                        .and_then(|other| other.tags().last().cloned());
                    if let Some(tag) = last {
                        self.remove_task_tag(task, tag, cx);
                    }
                } else if let Some(edit) = self.tag_edit.as_mut() {
                    edit.draft.pop();
                    edit.error = None;
                    self.refresh_suggestions(cx);
                }
            }
            _ => {
                if modifiers.control || modifiers.alt || modifiers.platform {
                    return false;
                }
                let Some(text) = typed_text(&event.keystroke) else {
                    return false;
                };
                self.type_tag_text(text, cx);
            }
        }
        cx.notify();
        true
    }

    fn step_highlight(&mut self, down: bool) {
        if let Some(edit) = self.tag_edit.as_mut() {
            edit.highlight = move_highlight(edit.highlight, edit.suggestions.len(), down);
        }
    }

    /// Reloads the suggestions for the draft from the tags in use. Nothing is suggested for an empty
    /// draft. Tags the task already has are left out.
    fn refresh_suggestions(&mut self, cx: &mut Context<Self>) {
        let Some(edit) = self.tag_edit.as_ref() else {
            return;
        };
        let task = edit.task;
        let prefix = normalize_tag(&edit.draft).unwrap_or_default();
        let on_task: Vec<String> = self
            .board
            .iter()
            .find(|other| other.id() == task)
            .map(|other| other.tags().to_vec())
            .unwrap_or_default();
        let suggestions = match self.projects.active_id() {
            Some(project) if !prefix.is_empty() => {
                match store(cx).tags_in_use(project, &prefix, MAX_SUGGESTIONS) {
                    Ok(in_use) => offered(in_use, &on_task),
                    Err(err) => {
                        eprintln!("could not load tag suggestions: {err}");
                        Vec::new()
                    }
                }
            }
            _ => Vec::new(),
        };
        if let Some(edit) = self.tag_edit.as_mut() {
            edit.suggestions = suggestions;
            edit.highlight = None;
        }
    }

    /// Adds a tag to a task, normalised by the domain. Shared by typing, Enter, and blur.
    fn add_tag_text(
        &mut self,
        task: TaskId,
        text: &str,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let text = text.to_string();
        self.edit_task(task, |task| task.add_tag(&text), cx)
    }

    /// Shows a tag error in the input when it's open, or on the board's status line otherwise.
    fn set_tag_error(&mut self, message: String, cx: &mut Context<Self>) {
        match self.tag_edit.as_mut() {
            Some(edit) => edit.error = Some(message),
            None => self.status = Some(message),
        }
        cx.notify();
    }

    /// Clears the editors when their task is gone from the board.
    pub(super) fn close_tag_editors_if_gone(&mut self) {
        let on_board = |task: TaskId| self.board.iter().any(|other| other.id() == task);
        if self.card_editing.is_some_and(|task| !on_board(task)) {
            self.card_editing = None;
        }
        if self
            .tag_edit
            .as_ref()
            .is_some_and(|edit| !on_board(edit.task))
        {
            self.tag_edit = None;
        }
    }
}

#[cfg(test)]
mod tests {
    // Not `use super::*`: gpui's `test` macro would shadow `#[test]`.
    use super::{MAX_SUGGESTIONS, move_highlight, offered};

    fn tags(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| name.to_string()).collect()
    }

    #[test]
    fn suggestions_leave_out_tags_the_task_already_has() {
        assert_eq!(
            offered(tags(&["alpha", "beta", "gamma"]), &tags(&["beta"])),
            ["alpha", "gamma"]
        );
    }

    #[test]
    fn the_suggestion_limit_is_eight() {
        assert_eq!(MAX_SUGGESTIONS, 8);
    }

    #[test]
    fn the_highlight_moves_down_and_stops_at_the_last_suggestion() {
        assert_eq!(move_highlight(None, 3, true), Some(0));
        assert_eq!(move_highlight(Some(1), 3, true), Some(2));
        assert_eq!(move_highlight(Some(2), 3, true), Some(2));
    }

    #[test]
    fn the_highlight_moves_up_and_stepping_past_the_first_clears_it() {
        assert_eq!(move_highlight(None, 3, false), Some(2));
        assert_eq!(move_highlight(Some(2), 3, false), Some(1));
        assert_eq!(move_highlight(Some(0), 3, false), None);
    }

    #[test]
    fn nothing_is_highlighted_without_suggestions() {
        assert_eq!(move_highlight(None, 0, true), None);
        assert_eq!(move_highlight(Some(0), 0, false), None);
    }
}
