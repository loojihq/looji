//! A multi-line text area that wraps long lines, grows to a limit, and scrolls to keep the caret in view.
//!
//! The text is laid out as a row of chunks that wraps: each word with the spaces after it is one chunk, and each
//! newline is a break that takes the whole row. The chunks are the scroll container's own children, so it can
//! scroll to the chunk the caret is in, which is how typing at the end of a long text stays visible. `chunks` and
//! `caret_chunk` are pure, so the layout is the same wherever it's asked for.

use std::ops::Range;

use gpui::{prelude::*, *};

use crate::text_edit::TextEdit;
use crate::theme::Theme;

/// The height of a line with nothing on it.
const LINE_HEIGHT: f32 = 18.;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChunkKind {
    /// A word and the spaces after it.
    Word,
    /// A newline. It takes the rest of the row, so the next chunk starts a new line.
    Break,
    /// A line with nothing on it.
    Blank,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    pub range: Range<usize>,
    pub kind: ChunkKind,
}

/// Splits `text` into the chunks the area lays out, in order. Every byte of the text is in exactly one chunk
/// except the newlines, which are each in a `Break`.
pub fn chunks(text: &str) -> Vec<Chunk> {
    let mut chunks = Vec::new();
    let mut offset = 0;
    let lines: Vec<&str> = text.split('\n').collect();
    for (index, line) in lines.iter().enumerate() {
        let line_end = offset + line.len();
        if line.is_empty() {
            chunks.push(Chunk {
                range: offset..offset,
                kind: ChunkKind::Blank,
            });
        } else {
            let mut start = offset;
            let mut in_spaces = false;
            for (position, character) in line.char_indices() {
                let at = offset + position;
                if character == ' ' {
                    in_spaces = true;
                } else if in_spaces {
                    chunks.push(Chunk {
                        range: start..at,
                        kind: ChunkKind::Word,
                    });
                    start = at;
                    in_spaces = false;
                }
            }
            chunks.push(Chunk {
                range: start..line_end,
                kind: ChunkKind::Word,
            });
        }
        if index + 1 < lines.len() {
            chunks.push(Chunk {
                range: line_end..line_end + 1,
                kind: ChunkKind::Break,
            });
        }
        offset = line_end + 1;
    }
    chunks
}

/// The index of the chunk the caret is in, which is the one to scroll to. A caret between two words belongs to
/// the first, and one at the end of a line belongs to that line's last word.
pub fn caret_chunk(text: &str, caret: usize) -> usize {
    let all = chunks(text);
    all.iter()
        .position(|chunk| {
            chunk.kind != ChunkKind::Break && chunk.range.start <= caret && caret <= chunk.range.end
        })
        .unwrap_or_else(|| all.len().saturating_sub(1))
}

/// The text area. It takes the room its parent leaves, and no less than `min_height`, then scrolls when its text is
/// taller than it is.
pub fn edit_area(
    edit: &TextEdit,
    placeholder: &str,
    active: bool,
    min_height: f32,
    scroll: &ScrollHandle,
    theme: &Theme,
) -> Stateful<Div> {
    let frame = div()
        .id("edit-area")
        .flex()
        .flex_row()
        .flex_wrap()
        .items_start()
        .content_start()
        .w_full()
        .flex_1()
        .min_h(px(min_height))
        .px(theme.spacing.sm)
        .py(theme.spacing.sm)
        .rounded(theme.radius.md)
        .border_1()
        .border_color(if active {
            theme.palette.text_secondary
        } else {
            theme.palette.border
        })
        .bg(theme.palette.surface)
        .text_size(theme.typography.body)
        .overflow_y_scroll()
        .track_scroll(scroll);

    if edit.is_empty() && !active {
        return frame.child(
            div()
                .text_color(theme.palette.text_secondary)
                .child(placeholder.to_string()),
        );
    }

    let text = edit.as_str();
    let selection = edit.selection();
    let caret = edit.caret();
    let caret_at = caret_chunk(text, caret);
    let mut area = frame;
    for (index, chunk) in chunks(text).into_iter().enumerate() {
        let caret_here = active && index == caret_at;
        area = match chunk.kind {
            ChunkKind::Break => area.child(div().w_full().h(px(0.))),
            ChunkKind::Blank => {
                let line = div().w_full().h(px(LINE_HEIGHT)).flex().items_center();
                area.child(if caret_here {
                    line.child(caret_mark(theme))
                } else {
                    line
                })
            }
            ChunkKind::Word => area.child(word(
                text,
                &chunk.range,
                caret_here.then_some(caret),
                selection.as_ref(),
                theme,
            )),
        };
    }
    area
}

/// One word, split where the caret and the selection change so each piece is drawn as selected or not.
fn word(
    text: &str,
    range: &Range<usize>,
    caret: Option<usize>,
    selection: Option<&Range<usize>>,
    theme: &Theme,
) -> Div {
    let caret = caret.map(|at| at.clamp(range.start, range.end));
    let mut cuts = vec![range.start, range.end];
    cuts.extend(caret);
    if let Some(selected) = selection {
        cuts.push(selected.start.clamp(range.start, range.end));
        cuts.push(selected.end.clamp(range.start, range.end));
    }
    cuts.sort_unstable();
    cuts.dedup();

    let mut piece = div()
        .flex()
        .flex_row()
        .flex_none()
        .items_center()
        .min_h(px(LINE_HEIGHT));
    for pair in cuts.windows(2) {
        let (from, to) = (pair[0], pair[1]);
        if caret == Some(from) {
            piece = piece.child(caret_mark(theme));
        }
        let selected = selection.is_some_and(|chosen| from >= chosen.start && to <= chosen.end);
        let shown = text[from..to].to_string();
        piece = piece.child(if selected {
            div().bg(theme.palette.accent.opacity(0.35)).child(shown)
        } else {
            div().child(shown)
        });
    }
    if caret == Some(range.end) {
        piece = piece.child(caret_mark(theme));
    }
    piece
}

/// The caret: a thin upright bar in the text colour.
fn caret_mark(theme: &Theme) -> Div {
    div().w(px(1.)).h(px(16.)).bg(theme.palette.text_primary)
}

#[cfg(test)]
mod tests {
    use super::{ChunkKind, caret_chunk, chunks};

    fn kinds(text: &str) -> Vec<ChunkKind> {
        chunks(text).into_iter().map(|chunk| chunk.kind).collect()
    }

    #[test]
    fn words_keep_the_spaces_after_them_and_cover_the_text() {
        let text = "alpha  beta gamma";
        let found = chunks(text);
        let pieces: Vec<&str> = found
            .iter()
            .map(|chunk| &text[chunk.range.clone()])
            .collect();
        assert_eq!(pieces, ["alpha  ", "beta ", "gamma"]);
    }

    #[test]
    fn each_newline_is_a_break_and_an_empty_line_is_blank() {
        assert_eq!(
            kinds("a\n\nb"),
            [
                ChunkKind::Word,
                ChunkKind::Break,
                ChunkKind::Blank,
                ChunkKind::Break,
                ChunkKind::Word
            ]
        );
        assert_eq!(kinds(""), [ChunkKind::Blank]);
        assert_eq!(
            kinds("a\n"),
            [ChunkKind::Word, ChunkKind::Break, ChunkKind::Blank]
        );
    }

    #[test]
    fn the_caret_belongs_to_the_word_before_it_and_to_the_start_of_the_next_line() {
        let text = "one two\nthree";
        // At the end of the text, the last word. At the end of the first line, its last word.
        assert_eq!(caret_chunk(text, text.len()), 3);
        assert_eq!(caret_chunk(text, 7), 1);
        // At the start of the second line, that line's first word.
        assert_eq!(caret_chunk(text, 8), 3);
        // Between two words, the first.
        assert_eq!(caret_chunk(text, 4), 0);
    }

    #[test]
    fn multibyte_text_is_split_on_character_boundaries() {
        let text = "caf\u{e9} \u{1f600}\u{1f600} end";
        let found = chunks(text);
        assert!(found.iter().all(|chunk| {
            text.is_char_boundary(chunk.range.start) && text.is_char_boundary(chunk.range.end)
        }));
        assert_eq!(found.len(), 3);
    }
}
