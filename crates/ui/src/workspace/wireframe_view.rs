//! A wireframe in a blueprint document, drawn from its element tree with the theme's colours. Rows share their width,
//! columns stack, and every other element is a plain box with its label, so it reads as a sketch of a screen.

use gpui::{prelude::*, *};
use looji_diagram::{Element, ElementKind, Wireframe};

use super::{WorkspaceView, task_dialog::button};
use crate::theme::Theme;

/// The wireframe with its title and save buttons above a framed sketch. `doc` and `index` say which one it is.
pub(super) fn block(
    wireframe: &Wireframe,
    doc: usize,
    index: usize,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> AnyElement {
    let id = doc * 100 + index;
    div()
        .flex()
        .flex_col()
        .gap(theme.spacing.xs)
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_size(theme.typography.caption)
                        .text_color(theme.palette.text_secondary)
                        .child(if wireframe.title.is_empty() {
                            "Wireframe".to_string()
                        } else {
                            format!("Wireframe: {}", wireframe.title)
                        }),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .gap(theme.spacing.xs)
                        .child(
                            button(("wireframe-svg", id), "Save SVG", false, theme).on_click(
                                cx.listener(move |this, _: &ClickEvent, _, cx| {
                                    this.save_wireframe(doc, index, false, cx)
                                }),
                            ),
                        )
                        .child(
                            button(("wireframe-png", id), "Save PNG", false, theme).on_click(
                                cx.listener(move |this, _: &ClickEvent, _, cx| {
                                    this.save_wireframe(doc, index, true, cx)
                                }),
                            ),
                        ),
                ),
        )
        .child(
            div()
                .max_w(px(560.))
                .p(theme.spacing.md)
                .rounded(theme.radius.md)
                .border_1()
                .border_color(theme.palette.border)
                .bg(theme.palette.surface)
                .child(draw(&wireframe.root, theme)),
        )
        .into_any_element()
}

fn draw(element: &Element, theme: &Theme) -> AnyElement {
    let label = element.label.clone();
    match element.kind {
        ElementKind::Column | ElementKind::Row | ElementKind::Card => {
            let mut container = div().flex().gap(theme.spacing.sm).min_w_0();
            container = if element.kind == ElementKind::Row {
                container.flex_row().items_start()
            } else {
                container.flex_col()
            };
            if element.kind == ElementKind::Card {
                container = container
                    .p(theme.spacing.sm)
                    .rounded(theme.radius.md)
                    .border_1()
                    .border_color(theme.palette.border_control)
                    .bg(theme.palette.surface_raised);
            }
            let in_row = element.kind == ElementKind::Row;
            for child in &element.children {
                let drawn = draw(child, theme);
                // Side by side, each child takes an equal share of the row.
                container = container.child(if in_row {
                    div().flex_1().min_w_0().child(drawn).into_any_element()
                } else {
                    drawn
                });
            }
            container.into_any_element()
        }
        ElementKind::Heading => div()
            .text_size(theme.typography.heading)
            .font_weight(FontWeight::MEDIUM)
            .child(label)
            .into_any_element(),
        ElementKind::Text => div()
            .text_size(theme.typography.body)
            .text_color(theme.palette.text_secondary)
            .child(label)
            .into_any_element(),
        ElementKind::Button => div()
            .flex()
            .items_center()
            .justify_center()
            .px(theme.spacing.md)
            .py(theme.spacing.xs)
            .rounded(theme.radius.md)
            .border_1()
            .border_color(theme.palette.accent)
            .text_color(theme.palette.accent)
            .text_size(theme.typography.body)
            .child(label)
            .into_any_element(),
        ElementKind::Input => div()
            .flex()
            .items_center()
            .h(px(32.))
            .px(theme.spacing.sm)
            .rounded(theme.radius.md)
            .border_1()
            .border_color(theme.palette.border_control)
            .text_size(theme.typography.body)
            .text_color(theme.palette.text_secondary)
            .child(label)
            .into_any_element(),
        ElementKind::Image => div()
            .flex()
            .items_center()
            .justify_center()
            .h(px(96.))
            .rounded(theme.radius.md)
            .border_1()
            .border_color(theme.palette.border_control)
            .bg(theme.palette.surface_hover)
            .text_size(theme.typography.caption)
            .text_color(theme.palette.text_secondary)
            .child(if label.is_empty() {
                "image".to_string()
            } else {
                label
            })
            .into_any_element(),
        ElementKind::Divider => div()
            .h(px(1.))
            .w_full()
            .bg(theme.palette.border)
            .into_any_element(),
    }
}
