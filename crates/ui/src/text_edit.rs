//! A single-line text draft with a caret and a selection. Every text field keeps one of these, so the
//! arrow keys, Home and End, Shift to select, Ctrl+A to select everything, and typing over a selection
//! all behave the same way wherever the field is.

use std::ops::{Deref, Range};

use gpui::Keystroke;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TextEdit {
    text: String,
    /// Byte offset of the caret, always on a character boundary.
    caret: usize,
    /// Where a selection started. The selection runs from here to the caret, either way round.
    anchor: Option<usize>,
}

impl TextEdit {
    /// A draft holding `text`, with the caret at its end.
    pub fn new(text: impl Into<String>) -> Self {
        let text = text.into();
        let caret = text.len();
        Self {
            text,
            caret,
            anchor: None,
        }
    }

    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// The byte offset of the caret.
    pub fn caret(&self) -> usize {
        self.caret
    }

    /// The selected bytes, when there is a selection.
    pub fn selection(&self) -> Option<Range<usize>> {
        let anchor = self.anchor?;
        if anchor == self.caret {
            return None;
        }
        Some(anchor.min(self.caret)..anchor.max(self.caret))
    }

    /// Replaces the whole draft, with the caret at its end and nothing selected.
    pub fn set(&mut self, text: impl Into<String>) {
        *self = Self::new(text);
    }

    pub fn clear(&mut self) {
        self.set("");
    }

    /// Types `typed` in place of any selection.
    pub fn insert(&mut self, typed: &str) {
        self.delete_selection();
        self.text.insert_str(self.caret, typed);
        self.caret += typed.len();
    }

    /// Removes the selection, or the character before the caret.
    pub fn backspace(&mut self) {
        if self.delete_selection() {
            return;
        }
        if let Some(start) = self.previous_boundary(self.caret) {
            self.text.replace_range(start..self.caret, "");
            self.caret = start;
        }
    }

    /// Removes the selection, or the character after the caret.
    pub fn delete_forward(&mut self) {
        if self.delete_selection() {
            return;
        }
        if let Some(end) = self.next_boundary(self.caret) {
            self.text.replace_range(self.caret..end, "");
        }
    }

    /// Moves the caret one character left, or collapses a selection to its start.
    pub fn move_left(&mut self, extend: bool) {
        self.move_to(self.previous_boundary(self.caret).unwrap_or(0), extend);
    }

    /// Moves the caret one character right, or collapses a selection to its end.
    pub fn move_right(&mut self, extend: bool) {
        self.move_to(
            self.next_boundary(self.caret).unwrap_or(self.text.len()),
            extend,
        );
    }

    pub fn move_home(&mut self, extend: bool) {
        self.move_to(0, extend);
    }

    pub fn move_end(&mut self, extend: bool) {
        self.move_to(self.text.len(), extend);
    }

    pub fn select_all(&mut self) {
        self.anchor = Some(0);
        self.caret = self.text.len();
        if self.anchor == Some(self.caret) {
            self.anchor = None;
        }
    }

    /// Applies a key press to the draft. Returns false when the key isn't an editing key, so the
    /// caller can handle it, for example Enter or Escape.
    pub fn handle_key(&mut self, keystroke: &Keystroke) -> bool {
        let modifiers = &keystroke.modifiers;
        let shortcut = modifiers.control || modifiers.platform;
        let extend = modifiers.shift;
        match keystroke.key.as_str() {
            "backspace" => self.backspace(),
            "delete" => self.delete_forward(),
            "left" => self.move_left(extend),
            "right" => self.move_right(extend),
            "home" => self.move_home(extend),
            "end" => self.move_end(extend),
            "a" if shortcut => self.select_all(),
            _ => {
                if shortcut || modifiers.alt {
                    return false;
                }
                let typed = match keystroke.key_char.as_deref() {
                    Some(text) => text,
                    None if keystroke.key == "space" => " ",
                    None => return false,
                };
                self.insert(typed);
            }
        }
        true
    }

    fn move_to(&mut self, to: usize, extend: bool) {
        if extend {
            self.anchor.get_or_insert(self.caret);
        } else if let Some(range) = self.selection() {
            // Plain arrows collapse a selection to the side they point at.
            self.caret = if to < self.caret {
                range.start
            } else {
                range.end
            };
            self.anchor = None;
            return;
        } else {
            self.anchor = None;
        }
        self.caret = to;
        if self.anchor == Some(self.caret) {
            self.anchor = None;
        }
    }

    /// Removes the selected text. Returns whether there was a selection.
    fn delete_selection(&mut self) -> bool {
        let Some(range) = self.selection() else {
            self.anchor = None;
            return false;
        };
        self.text.replace_range(range.clone(), "");
        self.caret = range.start;
        self.anchor = None;
        true
    }

    fn previous_boundary(&self, from: usize) -> Option<usize> {
        self.text[..from]
            .char_indices()
            .last()
            .map(|(index, _)| index)
    }

    fn next_boundary(&self, from: usize) -> Option<usize> {
        self.text[from..]
            .chars()
            .next()
            .map(|character| from + character.len_utf8())
    }
}

impl Deref for TextEdit {
    type Target = str;

    fn deref(&self) -> &str {
        &self.text
    }
}

impl From<String> for TextEdit {
    fn from(text: String) -> Self {
        Self::new(text)
    }
}

#[cfg(test)]
mod tests {
    use super::TextEdit;

    #[test]
    fn typing_goes_at_the_caret() {
        let mut edit = TextEdit::new("ac");
        edit.move_left(false);
        edit.insert("b");
        assert_eq!(edit.as_str(), "abc");
        assert_eq!(edit.caret(), 2);
    }

    #[test]
    fn select_all_then_type_replaces_the_text() {
        let mut edit = TextEdit::new("old name");
        edit.select_all();
        assert_eq!(edit.selection(), Some(0..8));
        edit.insert("new");
        assert_eq!(edit.as_str(), "new");
        assert_eq!(edit.selection(), None);
    }

    #[test]
    fn backspace_removes_the_selection_or_one_character() {
        let mut edit = TextEdit::new("hello");
        edit.backspace();
        assert_eq!(edit.as_str(), "hell");
        edit.select_all();
        edit.backspace();
        assert_eq!(edit.as_str(), "");
        edit.backspace();
        assert_eq!(edit.as_str(), "");
    }

    #[test]
    fn shift_arrows_extend_and_plain_arrows_collapse() {
        let mut edit = TextEdit::new("abcd");
        edit.move_home(false);
        edit.move_right(true);
        edit.move_right(true);
        assert_eq!(edit.selection(), Some(0..2));
        edit.move_left(false);
        assert_eq!(edit.selection(), None);
        assert_eq!(edit.caret(), 0);
    }

    #[test]
    fn multibyte_characters_move_and_delete_whole() {
        let mut edit = TextEdit::new("é!");
        edit.move_left(false);
        edit.backspace();
        assert_eq!(edit.as_str(), "!");
        edit.move_home(false);
        edit.delete_forward();
        assert_eq!(edit.as_str(), "");
    }
}
