//! Shared controls. Every screen builds its buttons, inputs, chips, and rows from these, so they
//! look the same everywhere and a change here changes the whole app.
//!
//! Each one takes the `Theme` and reads only palette roles, spacing, radius, and type sizes. None
//! of them spells out a colour, which `theme::guard` checks.

use gpui::{prelude::*, *};
use looji_domain::MemberColour;

use crate::{text_edit::TextEdit, theme::Theme};

// ───────────────────────────── buttons ─────────────────────────────

/// How a button reads: the main action, an outlined secondary action, a quiet text action, or a
/// destructive one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Variant {
    Primary,
    Secondary,
    Ghost,
    Danger,
}

/// A text button. Give it an id so it can take a click; the caller adds `.on_click`.
pub fn button(
    id: impl Into<ElementId>,
    label: &str,
    variant: Variant,
    theme: &Theme,
) -> Stateful<Div> {
    shell(id, variant, theme).child(label.to_string())
}

/// A text button with a Tabler icon before its label. The icon takes the label's colour.
pub fn button_icon(
    id: impl Into<ElementId>,
    icon_name: &str,
    label: &str,
    variant: Variant,
    theme: &Theme,
) -> Stateful<Div> {
    // The icon takes the label's colour.
    let label_colour = match variant {
        Variant::Primary => theme.palette.on_accent,
        Variant::Secondary => theme.palette.text_primary,
        Variant::Ghost => theme.palette.text_secondary,
        Variant::Danger => theme.palette.on_danger,
    };
    let content = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(theme.spacing.xs)
        .child(icon(icon_name, 15., label_colour))
        .child(label.to_string());
    shell(id, variant, theme).child(content)
}

/// The button's shape and colours, before its label is added.
fn shell(id: impl Into<ElementId>, variant: Variant, theme: &Theme) -> Stateful<Div> {
    // Every variant has the same height, corner radius, weight, and 1px edge, so buttons sit in one row
    // without jumping. The edge matches the fill for the filled variants, so it doesn't show.
    let base = div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .h(px(32.))
        .px(theme.spacing.md)
        .rounded(theme.radius.lg)
        .border_1()
        .text_size(theme.typography.body)
        .font_weight(FontWeight::MEDIUM)
        .whitespace_nowrap()
        .cursor_pointer();
    match variant {
        Variant::Primary => base
            .bg(theme.palette.accent)
            .border_color(theme.palette.accent)
            .text_color(theme.palette.on_accent)
            .hover(|style| style.opacity(0.9)),
        // Filled rather than outlined, so only the primary action stands out. The edge matches the fill.
        Variant::Secondary => base
            .bg(theme.palette.surface_raised)
            .border_color(theme.palette.surface_raised)
            .text_color(theme.palette.text_primary)
            .hover(|style| style.bg(theme.palette.surface_hover)),
        Variant::Ghost => base
            .border_color(theme.palette.surface)
            .text_color(theme.palette.text_secondary)
            .hover(|style| {
                style
                    .bg(theme.palette.surface_hover)
                    .text_color(theme.palette.text_primary)
            }),
        Variant::Danger => base
            .bg(theme.palette.danger)
            .border_color(theme.palette.danger)
            .text_color(theme.palette.on_danger)
            .hover(|style| style.opacity(0.9)),
    }
}

/// A square button with one Tabler icon, such as "x" to close or "dots" for more.
pub fn icon_button(id: impl Into<ElementId>, icon_name: &str, theme: &Theme) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(26.))
        .rounded(theme.radius.md)
        .text_size(theme.typography.title)
        .text_color(theme.palette.text_secondary)
        .cursor_pointer()
        .hover(|style| {
            style
                .bg(theme.palette.surface_hover)
                .text_color(theme.palette.text_primary)
        })
        .child(icon(icon_name, 14., theme.palette.text_secondary))
}

// ───────────────────────────── inputs ─────────────────────────────

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

/// An editable single-line box. It shows the draft's text, with the caret where it sits and any
/// selection highlighted. An empty one shows the placeholder while it isn't being edited.
pub fn edit_box(edit: &TextEdit, placeholder: &str, active: bool, theme: &Theme) -> Div {
    let body = edit_text(edit, placeholder, active, theme);

    div()
        .flex()
        .items_center()
        .min_h(px(32.))
        .px(theme.spacing.sm)
        .rounded(theme.radius.md)
        .border_1()
        .border_color(if active {
            theme.palette.text_secondary
        } else {
            theme.palette.border_control
        })
        .bg(theme.palette.surface)
        .text_size(theme.typography.body)
        .child(body)
}

/// The text of an editable line: the caret where it sits, or the selection, or the placeholder.
fn edit_text(edit: &TextEdit, placeholder: &str, active: bool, theme: &Theme) -> Div {
    if edit.is_empty() && !active {
        div()
            .text_color(theme.palette.text_secondary)
            .child(placeholder.to_string())
    } else {
        let text = edit.as_str();
        let line = div().flex().flex_row().items_center().min_w_0().truncate();
        match edit.selection() {
            Some(range) => line
                .child(text[..range.start].to_string())
                .child(
                    div()
                        .bg(theme.palette.accent.opacity(0.35))
                        .child(text[range.clone()].to_string()),
                )
                .child(text[range.end..].to_string()),
            None => {
                let caret = edit.caret();
                let line = line.child(text[..caret].to_string());
                let line = if active {
                    line.child(caret_mark(theme))
                } else {
                    line
                };
                line.child(text[caret..].to_string())
            }
        }
    }
}

/// An editable box of several lines, such as a plan's goal. The caret and any selection show where
/// they sit, across the lines.
pub fn edit_lines(
    edit: &TextEdit,
    placeholder: &str,
    active: bool,
    min_height: f32,
    theme: &Theme,
) -> Div {
    let body = if edit.is_empty() && !active {
        div()
            .text_color(theme.palette.text_secondary)
            .child(placeholder.to_string())
    } else {
        let mut lines = div().flex().flex_col();
        let mut start = 0;
        for line in edit.as_str().split('\n') {
            let end = start + line.len();
            lines = lines.child(edit_row(edit, start, end, active, theme));
            start = end + 1;
        }
        lines
    };
    div()
        .h_full()
        .min_h(px(min_height))
        .w_full()
        .px(theme.spacing.sm)
        .py(theme.spacing.sm)
        .rounded(theme.radius.md)
        .border_1()
        .border_color(theme.palette.border_control)
        .bg(theme.palette.surface_raised)
        .text_size(theme.typography.body)
        .child(body)
}

/// One line of a multi-line edit: the bytes from `start` to `end`, split where the caret and the
/// selection change, so each piece can be drawn as selected or not.
fn edit_row(edit: &TextEdit, start: usize, end: usize, active: bool, theme: &Theme) -> Div {
    let text = edit.as_str();
    let caret = edit.caret();
    let caret_here = active && (start..=end).contains(&caret);
    let selection = edit.selection();

    let mut cuts = vec![start, end];
    if caret_here {
        cuts.push(caret);
    }
    if let Some(range) = &selection {
        cuts.push(range.start.clamp(start, end));
        cuts.push(range.end.clamp(start, end));
    }
    cuts.sort_unstable();
    cuts.dedup();

    let mut row = div()
        .flex()
        .flex_row()
        .items_center()
        .min_h(px(18.))
        .min_w_0();
    for pair in cuts.windows(2) {
        let (from, to) = (pair[0], pair[1]);
        if caret_here && caret == from {
            row = row.child(caret_mark(theme));
        }
        let selected = selection
            .as_ref()
            .is_some_and(|range| from >= range.start && to <= range.end);
        let piece = text[from..to].to_string();
        row = row.child(if selected {
            div().bg(theme.palette.accent.opacity(0.35)).child(piece)
        } else {
            div().child(piece)
        });
    }
    if caret_here && caret == end {
        row = row.child(caret_mark(theme));
    }
    row
}
/// The caret: a thin upright bar in the text colour.
fn caret_mark(theme: &Theme) -> Div {
    div().w(px(1.)).h(px(16.)).bg(theme.palette.text_primary)
}

/// A Tabler icon from the set compiled into the app, by name such as "search". It is drawn in the
/// colour it's given, so it follows the text it sits beside.
pub fn icon(name: &str, size: f32, color: Hsla) -> Svg {
    svg()
        .path(format!("icons/{name}.svg"))
        .size(px(size))
        .flex_none()
        .text_color(color)
}

/// The tick beside a chosen option in a menu. The slot keeps its width when the option isn't chosen,
/// so the labels stay in a column.
pub fn check_mark(checked: bool, theme: &Theme) -> Div {
    let slot = div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(14.));
    if checked {
        slot.child(icon("check", 14., theme.palette.text_primary))
    } else {
        slot
    }
}

/// A rounded pill, used for filters and toggles on a bar. `active` gives it the accent outline,
/// which marks the one whose menu is open. The caller adds the click and any key handling.
pub fn pill(id: impl Into<ElementId>, label: &str, active: bool, theme: &Theme) -> Stateful<Div> {
    // Filled, not outlined. The active one takes the stronger selected fill and a medium weight.
    div()
        .id(id)
        .px(theme.spacing.md)
        .py(theme.spacing.xs)
        .rounded_full()
        .text_size(theme.typography.caption)
        .text_color(theme.palette.text_primary)
        .cursor_pointer()
        .when(active, |pill| {
            pill.bg(theme.palette.surface_selected)
                .font_weight(FontWeight::MEDIUM)
        })
        .when(!active, |pill| {
            pill.bg(theme.palette.surface_raised)
                .hover(|style| style.bg(theme.palette.surface_hover))
        })
        .child(label.to_string())
}

/// A search field: a filled box with a search icon. It shows an accent edge only while it has the
/// keyboard, so the focus is visible without an outline on every field.
pub fn search_field(value: &TextEdit, placeholder: &str, active: bool, theme: &Theme) -> Div {
    let text = edit_text(value, placeholder, active, theme);
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(theme.spacing.sm)
        .h(px(32.))
        .px(theme.spacing.md)
        .rounded(theme.radius.lg)
        .bg(theme.palette.surface_raised)
        .border_1()
        .border_color(if active {
            theme.palette.accent
        } else {
            theme.palette.surface_raised
        })
        .text_size(theme.typography.body)
        .child(icon("search", 16., theme.palette.text_secondary))
        .child(div().flex_1().min_w_0().truncate().child(text))
}

/// A dropdown look: the current value and a chevron. The caller adds the click.
pub fn select(id: impl Into<ElementId>, label: &str, open: bool, theme: &Theme) -> Stateful<Div> {
    // Filled like the other controls. Open, it takes the selected fill, as a menu button does while its menu is up.
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .justify_between()
        .gap(theme.spacing.sm)
        .h(px(32.))
        .px(theme.spacing.md)
        .rounded(theme.radius.lg)
        .text_size(theme.typography.body)
        .text_color(theme.palette.text_primary)
        .cursor_pointer()
        .border_1()
        .border_color(theme.palette.border_control)
        .when(open, |select| select.bg(theme.palette.surface_selected))
        .when(!open, |select| {
            select
                .bg(theme.palette.surface_raised)
                .hover(|style| style.bg(theme.palette.surface_hover))
        })
        .child(div().min_w_0().truncate().child(label.to_string()))
        .child(icon("chevron-down", 14., theme.palette.text_secondary))
}

/// An on/off switch. The track is accent when on.
pub fn toggle(on: bool, theme: &Theme) -> Div {
    div()
        .relative()
        .flex_none()
        .w(px(34.))
        .h(px(20.))
        .rounded_full()
        .bg(if on {
            theme.palette.accent
        } else {
            theme.palette.border_control
        })
        .child(
            div()
                .absolute()
                .top(px(2.))
                .left(px(if on { 16. } else { 2. }))
                .size(px(16.))
                .rounded_full()
                .bg(theme.palette.surface),
        )
}

/// A tick box for a task. Filled when done, outlined otherwise.
pub fn checkbox(done: bool, theme: &Theme) -> Div {
    let base = div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(15.))
        .rounded_full()
        .text_size(theme.typography.caption);
    if done {
        base.bg(theme.palette.text_secondary)
            .text_color(theme.palette.surface)
            .child(icon("check", 10., theme.palette.surface))
    } else {
        base.border_1().border_color(theme.palette.border_control)
    }
}

// ───────────────────────────── labels and marks ─────────────────────────────

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

/// A keyboard shortcut, such as Esc or Ctrl F.
pub fn kbd(text: &str, theme: &Theme) -> Div {
    div()
        .px(px(5.))
        .py(px(1.))
        .rounded(theme.radius.sm)
        .border_1()
        .border_color(theme.palette.border_control)
        .text_size(theme.typography.caption)
        .text_color(theme.palette.text_secondary)
        .child(text.to_string())
}

/// A small heading over a group of rows, such as "Projects" or "Overdue".
pub fn section_label(text: &str, theme: &Theme) -> Div {
    div()
        .mt(theme.spacing.md)
        .mb(theme.spacing.xs)
        .text_size(theme.typography.caption)
        .text_color(theme.palette.text_secondary)
        .child(text.to_string())
}

/// A round dot, used to mark a project or an open state.
pub fn dot(color: Hsla, size: f32) -> Div {
    div().flex_none().size(px(size)).rounded_full().bg(color)
}

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

/// A thin bar showing a fraction from 0 to 1, such as how full a day is.
pub fn progress(fraction: f32, theme: &Theme) -> Div {
    div()
        .h(px(4.))
        .rounded_full()
        .bg(theme.palette.border)
        .overflow_hidden()
        .child(
            div()
                .h_full()
                .w(relative(fraction.clamp(0., 1.)))
                .bg(theme.palette.accent),
        )
}

// ───────────────────────────── rows and navigation ─────────────────────────────

/// One row of a task's properties: a fixed-width label, then the value.
pub fn prop_row(label: &str, value: impl IntoElement, theme: &Theme) -> Div {
    div()
        .flex()
        .flex_row()
        .items_center()
        .py(theme.spacing.xs)
        .child(
            div()
                .w(px(84.))
                .flex_none()
                .text_color(theme.palette.text_secondary)
                .child(label.to_string()),
        )
        .child(div().flex_1().min_w_0().child(value))
}

/// A square tile with one Tabler icon, used to introduce an empty screen.
pub fn icon_tile(icon_name: &str, theme: &Theme) -> Div {
    div()
        .flex()
        .items_center()
        .justify_center()
        .size(px(40.))
        .rounded(theme.radius.lg)
        .bg(theme.palette.surface_raised)
        .child(icon(icon_name, 20., theme.palette.text_secondary))
}

/// One numbered step under an empty screen: a number, a heading, and a line of explanation.
pub fn numbered_step(number: usize, heading: &str, body: &str, theme: &Theme) -> Div {
    div()
        .flex()
        .flex_row()
        .items_start()
        .gap(theme.spacing.md)
        .py(theme.spacing.sm)
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(px(22.))
                .rounded_full()
                .bg(theme.palette.surface_selected)
                .text_size(theme.typography.caption)
                .text_color(theme.palette.text_secondary)
                .child(number.to_string()),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .child(
                    div()
                        .font_weight(FontWeight::MEDIUM)
                        .child(heading.to_string()),
                )
                .child(
                    div()
                        .text_size(theme.typography.caption)
                        .text_color(theme.palette.text_secondary)
                        .child(body.to_string()),
                ),
        )
}

/// A sidebar row with an optional glyph and trailing text. Selected rows are highlighted.
pub fn nav_item(
    id: impl Into<ElementId>,
    glyph: &str,
    label: &str,
    selected: bool,
    trailing: Option<&str>,
    theme: &Theme,
) -> Stateful<Div> {
    let mut row = div()
        .id(id)
        .flex()
        .flex_row()
        .items_center()
        .gap(theme.spacing.sm)
        .px(theme.spacing.sm)
        .py(theme.spacing.xs)
        .rounded(theme.radius.md)
        .cursor_pointer()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .text_color(theme.palette.text_secondary)
        .when(selected, |row| {
            row.bg(theme.palette.surface_selected)
                .text_color(theme.palette.text_primary)
                .font_weight(FontWeight::MEDIUM)
        })
        .when(!selected, |row| {
            row.hover(|style| style.bg(theme.palette.surface_hover))
        })
        .child(icon(
            glyph,
            16.,
            if selected {
                theme.palette.text_primary
            } else {
                theme.palette.text_secondary
            },
        ))
        .child(div().flex_1().min_w_0().truncate().child(label.to_string()));
    if let Some(text) = trailing {
        row = row.child(
            div()
                .text_size(theme.typography.caption)
                .text_color(theme.palette.text_disabled)
                .child(text.to_string()),
        );
    }
    row
}

/// One setting or field row: its name and a line of explanation on the left, its control on the right, and a
/// rule under it. The caller makes the whole row clickable when the control is a toggle or select.
pub fn settings_row(
    name: &str,
    description: Option<&str>,
    control: impl IntoElement,
    theme: &Theme,
) -> Div {
    let mut text = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .child(name.to_string());
    if let Some(description) = description {
        text = text.child(
            div()
                .mt(px(1.))
                .text_size(theme.typography.caption)
                .text_color(theme.palette.text_secondary)
                .child(description.to_string()),
        );
    }
    div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(theme.spacing.md)
        .px(theme.spacing.sm)
        .py(theme.spacing.sm)
        .rounded(theme.radius.md)
        .border_b_1()
        .border_color(theme.palette.border)
        .child(text)
        .child(control)
}

/// A dropdown's floating menu, drawn over the page below its trigger. It is placed absolutely, so it
/// takes no room in the layout and moves nothing else. Wrap it in `deferred` and put it in a `relative`
/// element with its trigger, as the workspace switcher does.
pub fn dropdown_menu(
    id: impl Into<ElementId>,
    rows: Vec<AnyElement>,
    theme: &Theme,
) -> Stateful<Div> {
    div()
        .id(id)
        .absolute()
        .top(px(36.))
        .left_0()
        .w_full()
        .flex()
        .flex_col()
        .gap(theme.spacing.xs)
        .p(theme.spacing.xs)
        .rounded(theme.radius.lg)
        .bg(theme.palette.surface)
        .border_1()
        .border_color(theme.palette.border)
        .shadow(theme.elevation.overlay.clone())
        .occlude()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .children(rows)
}

/// One option in a dropdown menu. The chosen one is ringed in the accent colour, and the highlighted one
/// (the one the keys are on) takes the hover fill. The caller adds the click.
pub fn dropdown_row(
    id: impl Into<ElementId>,
    label: &str,
    selected: bool,
    highlighted: bool,
    theme: &Theme,
) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_row()
        .items_center()
        .px(theme.spacing.sm)
        .py(theme.spacing.xs)
        .rounded(theme.radius.md)
        .cursor_pointer()
        .border_1()
        .when(selected, |row| {
            row.bg(theme.palette.surface_selected)
                .border_color(theme.palette.accent)
        })
        .when(!selected, |row| {
            row.border_color(theme.palette.surface)
                .hover(|style| style.bg(theme.palette.surface_hover))
        })
        .when(highlighted && !selected, |row| {
            row.bg(theme.palette.surface_hover)
        })
        .child(div().min_w_0().truncate().child(label.to_string()))
}
