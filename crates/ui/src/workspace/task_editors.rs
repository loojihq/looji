//! The priority picker and the tag editor, drawn for a card or the panel. Both surfaces draw the same
//! controls and write through the same path, `WorkspaceView::edit_task`.
//!
//! Each control sits inside `contain`, which takes the press. A press on an editor therefore never
//! reaches the card under it, so it can't start a drag.

use gpui::{prelude::*, *};
use looji_domain::{Priority, Task};

use super::WorkspaceView;
use super::fields::{chip, segment};
use super::task_edit::TagEdit;
use crate::{components::edit_box, text_edit::TextEdit, theme::Theme};

const PRIORITY_CHOICES: [Option<Priority>; 4] = [
    None,
    Some(Priority::Low),
    Some(Priority::Medium),
    Some(Priority::High),
];

pub(super) fn priority_label(priority: Option<Priority>) -> &'static str {
    match priority {
        None => "None",
        Some(Priority::Low) => "Low",
        Some(Priority::Medium) => "Medium",
        Some(Priority::High) => "High",
    }
}

/// Stops a press on an editor from reaching the card or the board behind it.
fn contain(editor: Div) -> Div {
    editor.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
}

/// The priority choices in a row. The one the task has is selected.
pub(super) fn priority_picker(
    scope: &'static str,
    task: &Task,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> Div {
    let id = task.id();
    let mut row = div().flex().flex_row().flex_wrap().gap(theme.spacing.xs);
    for (index, value) in PRIORITY_CHOICES.into_iter().enumerate() {
        row = row.child(
            segment(task.priority() == value, theme)
                .id((scope, index as u64))
                .child(priority_label(value))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    // A failure is shown where the edit was made, so there's nothing to add here.
                    let _ = this.write_task_priority(id, value, cx);
                })),
        );
    }
    contain(row)
}

/// The tags as removable chips, then the input. The input is always there. Clicking it opens the
/// tag editor for this task, and while it's open, the suggestions show under it.
pub(super) fn tag_editor(
    scope: &'static str,
    task: &Task,
    edit: Option<&TagEdit>,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> Div {
    let id = task.id();
    let editing = edit.filter(|edit| edit.task == id);

    let mut chips = div().flex().flex_row().flex_wrap().gap(theme.spacing.xs);
    for (index, tag) in task.tags().iter().enumerate() {
        let remove = tag.clone();
        chips = chips.child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(theme.spacing.xs)
                .child(chip(tag, theme))
                .child(
                    div()
                        .id((scope, 100 + index as u64))
                        .px(theme.spacing.xs)
                        .cursor_pointer()
                        .text_color(theme.palette.text_secondary)
                        .child("×")
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.remove_task_tag(id, remove.clone(), cx)
                        })),
                ),
        );
    }

    let input = div()
        .id((scope, 0_u64))
        .child(edit_box(
            editing.map_or(&TextEdit::default(), |edit| &edit.draft),
            "Add a tag, then press Enter",
            editing.is_some(),
            theme,
        ))
        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
            this.focus_tag_input(id, window, cx)
        }));

    let mut editor = div()
        .flex()
        .flex_col()
        .gap(theme.spacing.xs)
        .child(chips)
        .child(input);

    if let Some(edit) = editing {
        if !edit.suggestions.is_empty() {
            let mut list = div()
                .flex()
                .flex_col()
                .gap(theme.spacing.xs)
                .p(theme.spacing.xs)
                .rounded(theme.radius.md)
                .border_1()
                .border_color(theme.palette.border_control)
                .bg(theme.palette.surface_raised);
            for (index, suggestion) in edit.suggestions.iter().enumerate() {
                let highlighted = edit.highlight == Some(index);
                list = list.child(
                    div()
                        .id((scope, 1000 + index as u64))
                        .px(theme.spacing.sm)
                        .py(theme.spacing.xs)
                        .rounded(theme.radius.sm)
                        .cursor_pointer()
                        .text_size(theme.typography.caption)
                        .when(highlighted, |row| row.bg(theme.palette.surface_selected))
                        .hover(|style| style.bg(theme.palette.surface_hover))
                        .child(suggestion.clone())
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.choose_suggestion(index, cx)
                        })),
                );
            }
            editor = editor.child(list);
        }
        if let Some(error) = edit.error.clone() {
            editor = editor.child(
                div()
                    .text_size(theme.typography.caption)
                    .text_color(theme.palette.danger)
                    .child(error),
            );
        }
    }

    contain(editor)
}
