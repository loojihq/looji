//! Markdown drawn in the app's own styles: headings, paragraphs, lists, code, tables, and rules. The blocks come from
//! `markdown::parse`, the same reader the chat uses, so a plan and a reply read the same way.

use gpui::{prelude::*, *};

use super::markdown::{Block, Inline, parse};
use crate::theme::Theme;

/// The markdown in `source`, as a column of blocks.
pub(super) fn view(source: &str, theme: &Theme) -> AnyElement {
    let mut column = div().flex().flex_col().gap(theme.spacing.sm);
    for block in parse(source) {
        column = column.child(block_view(&block, theme));
    }
    column.into_any_element()
}

fn block_view(block: &Block, theme: &Theme) -> AnyElement {
    match block {
        Block::Heading { level, content } => {
            let size = if *level <= 2 {
                theme.typography.heading
            } else {
                theme.typography.title
            };
            div()
                .text_size(size)
                .font_weight(FontWeight::MEDIUM)
                .child(flatten(content))
                .into_any_element()
        }
        Block::Paragraph(content) => div()
            .w_full()
            .line_height(relative(1.5))
            .child(flatten(content))
            .into_any_element(),
        Block::List { ordered, items } => {
            let mut list = div().flex().flex_col().gap(px(4.)).pl(theme.spacing.md);
            for (index, item) in items.iter().enumerate() {
                let marker = if *ordered {
                    format!("{}.", index + 1)
                } else {
                    "•".to_string()
                };
                list = list.child(
                    div()
                        .flex()
                        .flex_row()
                        .gap(theme.spacing.sm)
                        .child(
                            div()
                                .flex_none()
                                .text_color(theme.palette.text_secondary)
                                .child(marker),
                        )
                        .child(div().flex_1().min_w_0().child(flatten(item))),
                );
            }
            list.into_any_element()
        }
        Block::Code { text, .. } => div()
            .p(theme.spacing.sm)
            .rounded(theme.radius.md)
            .bg(theme.palette.surface_raised)
            .text_size(theme.typography.caption)
            .child(text.clone())
            .into_any_element(),
        Block::Table { header, rows } => {
            let mut lines: Vec<&[Vec<Inline>]> = vec![header.as_slice()];
            lines.extend(rows.iter().map(|row| row.as_slice()));
            let mut table = div().flex().flex_col();
            for (index, line) in lines.into_iter().enumerate() {
                let mut row = div()
                    .flex()
                    .flex_row()
                    .gap(theme.spacing.sm)
                    .py(theme.spacing.xs)
                    .border_b_1()
                    .border_color(theme.palette.border);
                for cell in line {
                    let text = flatten(cell);
                    row = row.child(div().flex_1().min_w_0().child(if index == 0 {
                        div()
                            .font_weight(FontWeight::MEDIUM)
                            .child(text)
                            .into_any_element()
                    } else {
                        text.into_any_element()
                    }));
                }
                table = table.child(row);
            }
            table.into_any_element()
        }
        Block::Rule => div()
            .h(px(1.))
            .w_full()
            .bg(theme.palette.border)
            .into_any_element(),
    }
}

/// The text of inline content with its styling left out, so it wraps as one block at the width it has.
fn flatten(content: &[Inline]) -> String {
    let mut out = String::new();
    for inline in content {
        match inline {
            Inline::Text(text) | Inline::Code(text) => out.push_str(text),
            Inline::Bold(children) | Inline::Italic(children) | Inline::Link { children, .. } => {
                out.push_str(&flatten(children));
            }
        }
    }
    out
}
