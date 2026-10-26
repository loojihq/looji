//! Draws the chat: the model row, each message, and the markdown inside a reply. Code blocks get a copy button, and
//! links open in the browser. Everything here reads the dialog's state; the actions are on `WorkspaceView`.

use std::path::Path;

use gpui::{prelude::*, *};
use looji_ai::{Source, models::MODELS};
use looji_domain::{ChatMessage, CodeSourceRepository, MessageRole, ProjectId, ReplyState};

use super::{
    WorkspaceView,
    chat::ChatDialog,
    markdown::{Block, Inline, parse},
    store,
    task_dialog::button,
};
use crate::theme::Theme;

const CODE_FONT: &str = "Cascadia Mono";

/// The model picker and the repository-code switch. The menu opens under the model's name.
pub(super) fn model_row(
    dialog: &ChatDialog,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> AnyElement {
    let picker = button(
        "chat-model",
        &format!("Model: {} (change)", dialog.model),
        false,
        theme,
    )
    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
        if let Some(dialog) = this.chat_dialog.as_mut() {
            dialog.model_menu = !dialog.model_menu;
        }
        cx.notify();
    }));

    let code = if dialog.code_available {
        let label = if dialog.use_code {
            "Repository code: on"
        } else {
            "Repository code: off"
        };
        button("chat-code", label, dialog.use_code, theme)
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.toggle_chat_code(cx)))
            .into_any_element()
    } else {
        div()
            .text_size(theme.typography.caption)
            .text_color(theme.palette.text_secondary)
            .child("Connect a repository to the project to use its code.")
            .into_any_element()
    };

    let mut row = div().flex().flex_col().gap(theme.spacing.xs).child(
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(theme.spacing.sm)
            .child(picker)
            .child(code),
    );
    if dialog.model_menu {
        let mut menu = div().flex().flex_row().flex_wrap().gap(theme.spacing.xs);
        for (index, model) in MODELS.iter().enumerate() {
            let chosen = dialog.model == *model;
            let model = model.to_string();
            menu = menu.child(
                button(("chat-model-option", index), model.as_str(), chosen, theme).on_click(
                    cx.listener(move |this, _: &ClickEvent, _, cx| this.set_chat_model(&model, cx)),
                ),
            );
        }
        row = row.child(menu);
    }
    row.into_any_element()
}

/// One message. A reply that is still streaming shows the text so far.
pub(super) fn message(
    dialog: &ChatDialog,
    message: &ChatMessage,
    window: &Window,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> AnyElement {
    let (label, text, state) = match message.role {
        MessageRole::User => (
            "You".to_string(),
            message.content.clone(),
            ReplyState::Complete,
        ),
        MessageRole::Assistant => {
            let (text, state) = match dialog.active.as_ref() {
                Some(active) if active.position == message.position => {
                    (active.reply.text.clone(), active.reply.state.clone())
                }
                _ => (message.content.clone(), message.state.clone()),
            };
            let model = message.model.clone().unwrap_or_default();
            (format!("Assistant · {model}"), text, state)
        }
    };

    let mut column = div().flex().flex_col().gap(theme.spacing.xs).child(
        div()
            .text_size(theme.typography.caption)
            .text_color(theme.palette.text_secondary)
            .child(label),
    );
    let body = match message.role {
        MessageRole::User => div().child(text).into_any_element(),
        MessageRole::Assistant => {
            let blocks = parse(&text);
            let mut content = div().flex().flex_col().gap(theme.spacing.sm);
            let mut code_index = 0usize;
            for (index, block) in blocks.iter().enumerate() {
                content = content.child(block_view(
                    block,
                    message.position,
                    index,
                    &mut code_index,
                    dialog,
                    window,
                    theme,
                    cx,
                ));
            }
            if text.is_empty() && state == ReplyState::Streaming {
                content = content.child(caption("Writing…", theme));
            }
            content.into_any_element()
        }
    };
    // While an agent works, what it is doing. The latest few steps are enough to see it moving.
    if message.role == MessageRole::Assistant
        && let Some(active) = dialog
            .active
            .as_ref()
            .filter(|active| active.position == message.position)
        && !active.steps.is_empty()
    {
        let mut steps = div().flex().flex_col().gap(px(2.));
        let skipped = active.steps.len().saturating_sub(4);
        for step in &active.steps[skipped..] {
            steps = steps.child(caption(&format!("\u{2022} {step}"), theme));
        }
        column = column.child(steps);
    }
    column = column.child(body);
    if message.role == MessageRole::Assistant {
        let sources = match dialog.active.as_ref() {
            Some(active) if active.position == message.position => Some(&active.sources),
            _ => dialog.sources.get(&message.position),
        };
        if let Some(sources) = sources.filter(|sources| !sources.is_empty()) {
            column = column.child(sources_used(sources, dialog.project, theme, cx));
        }
    }

    match &state {
        ReplyState::Streaming => {
            column = column.child(caption(
                "Writing… (press Stop to end the reply here)",
                theme,
            ))
        }
        ReplyState::Complete => {}
        ReplyState::Stopped => {
            column = column.child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(theme.spacing.sm)
                    .child(caption("Stopped. The text above is what arrived.", theme))
                    .child(retry_button(message.position, theme, cx)),
            );
        }
        ReplyState::Failed { reason } => {
            column = column.child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(theme.spacing.sm)
                    .child(
                        div()
                            .text_size(theme.typography.caption)
                            .text_color(theme.palette.danger)
                            .child(format!("The reply stopped early: {reason}")),
                    )
                    .child(retry_button(message.position, theme, cx)),
            );
        }
    }
    column.into_any_element()
}

/// What an answer drew on. Files are shown with their lines, and a click shows the file in the file manager.
fn sources_used(
    sources: &[Source],
    project: ProjectId,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> AnyElement {
    const SHOWN: usize = 8;
    let mut list = div()
        .flex()
        .flex_col()
        .gap(px(2.))
        .child(caption(&format!("Sources used ({})", sources.len()), theme))
        .child(caption(
            "Context sent: this conversation and the task details. Code was read only through the sources below.",
            theme,
        ));
    for (index, source) in sources.iter().take(SHOWN).enumerate() {
        let text = match source.kind.as_str() {
            "code" | "doc" if source.start_line > 0 => format!(
                "{}:{}-{}",
                source.reference, source.start_line, source.end_line
            ),
            "issue" => format!("Issue: {}", source.label),
            "commit" => format!(
                "Commit {}: {}",
                source.reference.chars().take(7).collect::<String>(),
                source.label
            ),
            _ => source.reference.clone(),
        };
        let row = div()
            .id(("chat-source", index))
            .text_size(theme.typography.caption)
            .text_color(theme.palette.accent)
            .child(text);
        list = list.child(if matches!(source.kind.as_str(), "code" | "doc") {
            let reference = source.reference.clone();
            row.cursor_pointer()
                .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                    if let Ok(Some(root)) = store(cx).load_path(project) {
                        cx.reveal_path(&Path::new(&root).join(&reference));
                    }
                }))
                .into_any_element()
        } else {
            row.into_any_element()
        });
    }
    if sources.len() > SHOWN {
        list = list.child(caption(
            &format!("and {} more", sources.len() - SHOWN),
            theme,
        ));
    }
    list.into_any_element()
}

fn caption(text: &str, theme: &Theme) -> impl IntoElement {
    div()
        .text_size(theme.typography.caption)
        .text_color(theme.palette.text_secondary)
        .child(text.to_string())
}

fn retry_button(position: u32, theme: &Theme, cx: &mut Context<WorkspaceView>) -> impl IntoElement {
    button(("chat-retry", position as usize), "Retry", false, theme).on_click(
        cx.listener(move |this, _: &ClickEvent, _, cx| this.retry_chat_reply(position, cx)),
    )
}

/// One markdown block. `code_index` counts code blocks through the message, so each copy button has its own key.
#[allow(clippy::too_many_arguments)]
fn block_view(
    block: &Block,
    position: u32,
    index: usize,
    code_index: &mut usize,
    dialog: &ChatDialog,
    window: &Window,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> AnyElement {
    let key = (position as usize) * 1_000 + index;
    match block {
        Block::Heading { level, content } => {
            let size = match level {
                1 => theme.typography.title,
                2 => theme.typography.title,
                _ => theme.typography.body,
            };
            div()
                .text_size(size)
                .font_weight(FontWeight::BOLD)
                .child(inline_text(content, ("heading", key), window, theme))
                .into_any_element()
        }
        Block::Paragraph(content) => inline_text(content, ("paragraph", key), window, theme),
        Block::List { ordered, items } => {
            let mut list = div().flex().flex_col().gap(theme.spacing.xs);
            for (number, item) in items.iter().enumerate() {
                let marker = if *ordered {
                    format!("{}.", number + 1)
                } else {
                    "•".to_string()
                };
                list = list.child(
                    div()
                        .flex()
                        .flex_row()
                        .gap(theme.spacing.sm)
                        .child(div().w(px(20.)).flex_shrink_0().child(marker))
                        .child(div().flex_1().min_w_0().child(inline_text(
                            item,
                            ("item", key * 100 + number),
                            window,
                            theme,
                        ))),
                );
            }
            list.into_any_element()
        }
        Block::Code { language, text } => {
            let block_index = *code_index;
            *code_index += 1;
            code_block(
                language.as_deref(),
                text,
                position,
                block_index,
                dialog,
                theme,
                cx,
            )
        }
        Block::Table { header, rows } => table(header, rows, key, window, theme),
        Block::Rule => div()
            .h(px(1.))
            .w_full()
            .bg(theme.palette.border)
            .into_any_element(),
    }
}

/// A fenced block: its language, a copy button that works with or without a language, and the code. Long lines
/// scroll sideways rather than widening the page.
fn code_block(
    language: Option<&str>,
    text: &str,
    position: u32,
    index: usize,
    dialog: &ChatDialog,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> AnyElement {
    let copied = dialog.copied == Some((position, index));
    let label = if copied { "Copied" } else { "Copy" };
    let code = text.to_string();
    let copy = button(
        ("copy-code", (position as usize) * 1_000 + index),
        label,
        false,
        theme,
    )
    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
        this.copy_code(position, index, code.clone(), cx)
    }));
    div()
        .flex()
        .flex_col()
        .rounded(theme.radius.md)
        .border_1()
        .border_color(theme.palette.border)
        .bg(theme.palette.surface)
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .px(theme.spacing.sm)
                .py(theme.spacing.xs)
                .child(caption(language.unwrap_or("code"), theme))
                .child(copy),
        )
        .child(
            div()
                .id(("code-body", (position as usize) * 1_000 + index))
                .overflow_x_scroll()
                .px(theme.spacing.sm)
                .py(theme.spacing.xs)
                .font_family(CODE_FONT)
                .text_size(theme.typography.body)
                .child(text.to_string()),
        )
        .into_any_element()
}

fn table(
    header: &[Vec<Inline>],
    rows: &[Vec<Vec<Inline>>],
    key: usize,
    window: &Window,
    theme: &Theme,
) -> AnyElement {
    let row_view = |cells: &[Vec<Inline>], id: usize, bold: bool| {
        let mut row = div()
            .flex()
            .flex_row()
            .border_b_1()
            .border_color(theme.palette.border);
        for (column, cell) in cells.iter().enumerate() {
            let mut text = inline_text(cell, ("cell", id * 100 + column), window, theme);
            if bold {
                text = div()
                    .font_weight(FontWeight::BOLD)
                    .child(text)
                    .into_any_element();
            }
            row = row.child(
                div()
                    .flex_1()
                    .min_w_0()
                    .px(theme.spacing.xs)
                    .py(theme.spacing.xs)
                    .child(text),
            );
        }
        row
    };
    let mut table = div()
        .flex()
        .flex_col()
        .rounded(theme.radius.md)
        .border_1()
        .border_color(theme.palette.border)
        .child(row_view(header, key * 10, true));
    for (index, cells) in rows.iter().enumerate() {
        table = table.child(row_view(cells, key * 10 + index + 1, false));
    }
    table.into_any_element()
}

/// Inline content as one text run with its styling. Links are clickable, and open in the browser.
fn inline_text(
    content: &[Inline],
    id: impl Into<ElementId>,
    window: &Window,
    theme: &Theme,
) -> AnyElement {
    let base = window.text_style().font();
    let mut pieces: Vec<Piece> = Vec::new();
    flatten(content, Style::default(), None, &mut pieces);

    let mut text = String::new();
    let mut runs = Vec::new();
    let mut links: Vec<(std::ops::Range<usize>, String)> = Vec::new();
    for piece in &pieces {
        if piece.text.is_empty() {
            continue;
        }
        let start = text.len();
        text.push_str(&piece.text);
        let mut font = base.clone();
        if piece.style.bold {
            font.weight = FontWeight::BOLD;
        }
        if piece.style.italic {
            font.style = FontStyle::Italic;
        }
        if piece.style.code {
            font.family = CODE_FONT.into();
        }
        let link = piece.link.is_some();
        runs.push(TextRun {
            len: piece.text.len(),
            font,
            color: if link {
                theme.palette.accent
            } else {
                theme.palette.text_primary
            },
            background_color: piece.style.code.then_some(theme.palette.surface_selected),
            underline: link.then(|| UnderlineStyle {
                thickness: px(1.),
                color: Some(theme.palette.accent),
                wavy: false,
            }),
            strikethrough: None,
        });
        if let Some(url) = &piece.link {
            links.push((start..text.len(), url.clone()));
        }
    }
    if runs.is_empty() {
        // Empty content still needs a run, or the text element has nothing to lay out.
        runs.push(TextRun {
            len: 0,
            font: base.clone(),
            color: theme.palette.text_primary,
            background_color: None,
            underline: None,
            strikethrough: None,
        });
    }
    let ranges: Vec<std::ops::Range<usize>> =
        links.iter().map(|(range, _)| range.clone()).collect();
    let styled = StyledText::new(text).with_runs(runs);
    InteractiveText::new(id, styled)
        .on_click(ranges, move |index, _, cx| {
            if let Some((_, url)) = links.iter().find(|(range, _)| range.contains(&index)) {
                cx.open_url(url);
            }
        })
        .into_any_element()
}

#[derive(Clone, Copy, Default)]
struct Style {
    bold: bool,
    italic: bool,
    code: bool,
}

struct Piece {
    text: String,
    style: Style,
    link: Option<String>,
}

/// Turns nested inline content into a flat list of styled pieces, so one text run can be built from it.
fn flatten(content: &[Inline], style: Style, link: Option<&str>, out: &mut Vec<Piece>) {
    for inline in content {
        match inline {
            Inline::Text(text) => out.push(Piece {
                text: text.clone(),
                style,
                link: link.map(str::to_owned),
            }),
            Inline::Code(code) => out.push(Piece {
                text: code.clone(),
                style: Style {
                    code: true,
                    ..style
                },
                link: link.map(str::to_owned),
            }),
            Inline::Bold(children) => flatten(
                children,
                Style {
                    bold: true,
                    ..style
                },
                link,
                out,
            ),
            Inline::Italic(children) => flatten(
                children,
                Style {
                    italic: true,
                    ..style
                },
                link,
                out,
            ),
            Inline::Link { children, url } => flatten(children, style, Some(url), out),
        }
    }
}
