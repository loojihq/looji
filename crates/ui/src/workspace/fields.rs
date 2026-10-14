//! Field pieces for the task dialog. They draw what they're given and keep no state, so the dialog
//! decides what's active and what's typed. They're generic, so the inline editor and the detail
//! panel can use the same ones.

use gpui::{prelude::*, *};
use looji_domain::MemberColour;

use crate::theme::Theme;

/// A team member's mark: their initials on their palette colour. The colour tells members apart
/// when two share a name, and the initials are the text the eye reads first.
pub fn member_mark(initials: &str, colour: MemberColour, size: Pixels, theme: &Theme) -> Div {
    let pair = theme.palette.avatars[colour.index()];
    div()
        .flex()
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .size(size)
        .rounded_full()
        .bg(pair.background)
        .text_color(pair.text)
        .text_size(theme.typography.caption)
        .child(initials.to_string())
}

/// A labelled field: the label above the input, and the error below it when there is one.
pub fn field(label: &str, error: Option<&str>, body: impl IntoElement, theme: &Theme) -> Div {
    let mut column = div()
        .flex()
        .flex_col()
        .gap(theme.spacing.xs)
        .child(
            div()
                .text_size(theme.typography.caption)
                .text_color(theme.palette.text_secondary)
                .child(label.to_string()),
        )
        .child(body);
    if let Some(error) = error {
        column = column.child(
            div()
                .text_size(theme.typography.caption)
                .text_color(theme.palette.danger)
                .child(error.to_string()),
        );
    }
    column
}

/// A single-line text box. The active one shows a caret, and an empty one shows the placeholder.
pub fn text_box(value: &str, placeholder: &str, active: bool, theme: &Theme) -> Div {
    let text = if value.is_empty() && !active {
        div()
            .text_color(theme.palette.text_secondary)
            .child(placeholder.to_string())
    } else {
        let caret = if active { "|" } else { "" };
        div().child(format!("{value}{caret}"))
    };
    div()
        .flex()
        .items_center()
        .min_h(px(32.))
        .px(theme.spacing.sm)
        .rounded(theme.radius.md)
        .border_1()
        .border_color(if active {
            theme.palette.accent
        } else {
            theme.palette.border_control
        })
        .bg(theme.palette.surface)
        .text_size(theme.typography.body)
        .child(text)
}

/// A chip: one tag, shown as a small rounded label.
pub fn chip(text: &str, theme: &Theme) -> Div {
    div()
        .px(theme.spacing.sm)
        .py(theme.spacing.xs)
        .rounded(theme.radius.sm)
        .bg(theme.palette.surface_selected)
        .text_size(theme.typography.caption)
        .text_color(theme.palette.text_primary)
        .child(text.to_string())
}

/// The base of one choice in a segmented control. The caller adds the label and the click.
pub fn segment(selected: bool, theme: &Theme) -> Div {
    let base = div()
        .flex()
        .items_center()
        .justify_center()
        .px(theme.spacing.sm)
        .py(theme.spacing.xs)
        .rounded(theme.radius.md)
        .border_1()
        .border_color(theme.palette.border_control)
        .text_size(theme.typography.caption)
        .cursor_pointer();
    if selected {
        base.bg(theme.palette.surface_selected)
            .text_color(theme.palette.text_primary)
    } else {
        base.text_color(theme.palette.text_secondary)
            .hover(|style| style.bg(theme.palette.surface_hover))
    }
}
