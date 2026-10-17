//! Reads a chat reply's markdown into blocks the chat view can draw. Parsing is pure, so it can be tested on sample
//! replies, including ones cut off mid-way, which is what a streaming reply looks like while it arrives. Anything
//! that can't be read as a known block is shown as plain text, so nothing a model writes can break the layout.

use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

/// Text with its styling. A link keeps its children, so bold text can be a link too.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Inline {
    Text(String),
    Bold(Vec<Inline>),
    Italic(Vec<Inline>),
    Code(String),
    Link { children: Vec<Inline>, url: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    Heading {
        level: u8,
        content: Vec<Inline>,
    },
    Paragraph(Vec<Inline>),
    List {
        ordered: bool,
        items: Vec<Vec<Inline>>,
    },
    /// A fenced block. `text` is the code without the fence lines. An unclosed fence runs to the end of the reply.
    Code {
        language: Option<String>,
        text: String,
    },
    Table {
        header: Vec<Vec<Inline>>,
        rows: Vec<Vec<Vec<Inline>>>,
    },
    Rule,
}

pub fn parse(source: &str) -> Vec<Block> {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    let events: Vec<Event> = Parser::new_ext(source, options).collect();
    let mut position = 0;
    blocks(&events, &mut position, None)
}

/// Reads blocks until the end of the events, or until the container's end tag when `until` is set.
fn blocks(
    events: &[Event],
    position: &mut usize,
    until: Option<&dyn Fn(&TagEnd) -> bool>,
) -> Vec<Block> {
    let mut out = Vec::new();
    while *position < events.len() {
        let event = &events[*position];
        if let (Event::End(end), Some(is_end)) = (event, until)
            && is_end(end)
        {
            *position += 1;
            return out;
        }
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                *position += 1;
                let content = inlines(events, position, &|end| matches!(end, TagEnd::Heading(_)));
                out.push(Block::Heading {
                    level: *level as u8,
                    content,
                });
            }
            Event::Start(Tag::Paragraph) => {
                *position += 1;
                let content = inlines(events, position, &|end| matches!(end, TagEnd::Paragraph));
                out.push(Block::Paragraph(content));
            }
            Event::Start(Tag::List(start)) => {
                let ordered = start.is_some();
                *position += 1;
                out.push(list(events, position, ordered));
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                let language = match kind {
                    CodeBlockKind::Fenced(info) => info
                        .split_whitespace()
                        .next()
                        .filter(|word| !word.is_empty())
                        .map(str::to_owned),
                    CodeBlockKind::Indented => None,
                };
                *position += 1;
                let mut text = String::new();
                while *position < events.len() {
                    match &events[*position] {
                        Event::Text(code) => text.push_str(code),
                        Event::End(TagEnd::CodeBlock) => break,
                        _ => {}
                    }
                    *position += 1;
                }
                *position += 1;
                let text = text.strip_suffix('\n').map(str::to_owned).unwrap_or(text);
                out.push(Block::Code { language, text });
            }
            Event::Start(Tag::Table(_)) => {
                *position += 1;
                out.push(table(events, position));
            }
            Event::Start(Tag::BlockQuote(_)) => {
                *position += 1;
                let inner = blocks(
                    events,
                    position,
                    Some(&|end| matches!(end, TagEnd::BlockQuote(_))),
                );
                out.extend(inner);
            }
            Event::Rule => {
                out.push(Block::Rule);
                *position += 1;
            }
            _ => *position += 1,
        }
    }
    out
}

fn list(events: &[Event], position: &mut usize, ordered: bool) -> Block {
    let mut items = Vec::new();
    while *position < events.len() {
        match &events[*position] {
            Event::Start(Tag::Item) => {
                *position += 1;
                items.push(item_text(events, position));
            }
            Event::End(TagEnd::List(_)) => {
                *position += 1;
                break;
            }
            _ => *position += 1,
        }
    }
    Block::List { ordered, items }
}

/// An item's text. Paragraphs inside it are joined with a line break, and a nested list is written as lines.
fn item_text(events: &[Event], position: &mut usize) -> Vec<Inline> {
    let mut content = Vec::new();
    while *position < events.len() {
        match &events[*position] {
            Event::End(TagEnd::Item) => {
                *position += 1;
                break;
            }
            Event::Start(Tag::Paragraph) => {
                *position += 1;
                if !content.is_empty() {
                    content.push(Inline::Text("\n".to_string()));
                }
                content.extend(inlines(events, position, &|end| {
                    matches!(end, TagEnd::Paragraph)
                }));
            }
            Event::Start(Tag::List(start)) => {
                let ordered = start.is_some();
                *position += 1;
                if let Block::List { items, .. } = list(events, position, ordered) {
                    for (index, item) in items.into_iter().enumerate() {
                        let marker = if ordered {
                            format!("\n{}. ", index + 1)
                        } else {
                            "\n• ".to_string()
                        };
                        content.push(Inline::Text(marker));
                        content.extend(item);
                    }
                }
            }
            _ => {
                if let Some(inline) = inline_event(events, position) {
                    content.push(inline);
                }
            }
        }
    }
    content
}

fn table(events: &[Event], position: &mut usize) -> Block {
    let mut header = Vec::new();
    let mut rows = Vec::new();
    while *position < events.len() {
        match &events[*position] {
            Event::Start(Tag::TableHead) => {
                *position += 1;
                header = cells(events, position, &|end| matches!(end, TagEnd::TableHead));
            }
            Event::Start(Tag::TableRow) => {
                *position += 1;
                rows.push(cells(events, position, &|end| {
                    matches!(end, TagEnd::TableRow)
                }));
            }
            Event::End(TagEnd::Table) => {
                *position += 1;
                break;
            }
            _ => *position += 1,
        }
    }
    Block::Table { header, rows }
}

/// The cells of one row or header, each read as its own inline run.
fn cells(
    events: &[Event],
    position: &mut usize,
    is_end: &dyn Fn(&TagEnd) -> bool,
) -> Vec<Vec<Inline>> {
    let mut out = Vec::new();
    while *position < events.len() {
        match &events[*position] {
            Event::Start(Tag::TableCell) => {
                *position += 1;
                out.push(inlines(events, position, &|end| {
                    matches!(end, TagEnd::TableCell)
                }));
            }
            Event::End(end) if is_end(end) => {
                *position += 1;
                break;
            }
            _ => *position += 1,
        }
    }
    out
}

/// Inline content until the end tag that `is_end` accepts, consumed. Nested styling is read recursively.
fn inlines(
    events: &[Event],
    position: &mut usize,
    is_end: &dyn Fn(&TagEnd) -> bool,
) -> Vec<Inline> {
    let mut out = Vec::new();
    while *position < events.len() {
        if let Event::End(end) = &events[*position]
            && is_end(end)
        {
            *position += 1;
            break;
        }
        if let Some(inline) = inline_event(events, position) {
            out.push(inline);
        }
    }
    out
}

/// One inline event, consuming what it spans. `None` for events that carry nothing to draw.
fn inline_event(events: &[Event], position: &mut usize) -> Option<Inline> {
    let event = &events[*position];
    match event {
        Event::Text(text) => {
            *position += 1;
            Some(Inline::Text(text.to_string()))
        }
        Event::Code(code) => {
            *position += 1;
            Some(Inline::Code(code.to_string()))
        }
        Event::SoftBreak => {
            *position += 1;
            Some(Inline::Text(" ".to_string()))
        }
        Event::HardBreak => {
            *position += 1;
            Some(Inline::Text("\n".to_string()))
        }
        Event::Start(Tag::Strong) => {
            *position += 1;
            Some(Inline::Bold(inlines(events, position, &|end| {
                matches!(end, TagEnd::Strong)
            })))
        }
        Event::Start(Tag::Emphasis) => {
            *position += 1;
            Some(Inline::Italic(inlines(events, position, &|end| {
                matches!(end, TagEnd::Emphasis)
            })))
        }
        Event::Start(Tag::Link { dest_url, .. }) => {
            let url = dest_url.to_string();
            *position += 1;
            let children = inlines(events, position, &|end| matches!(end, TagEnd::Link));
            Some(Inline::Link { children, url })
        }
        // Anything else (images, strikethrough, footnotes) keeps its text and loses its styling.
        _ => {
            *position += 1;
            None
        }
    }
}

/// The plain text of inline content. Tests compare against it.
#[cfg(test)]
pub fn plain(content: &[Inline]) -> String {
    let mut out = String::new();
    for inline in content {
        match inline {
            Inline::Text(text) | Inline::Code(text) => out.push_str(text),
            Inline::Bold(children) | Inline::Italic(children) => out.push_str(&plain(children)),
            Inline::Link { children, .. } => out.push_str(&plain(children)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    // Not `use super::*`: gpui's `test` macro would shadow `#[test]`.
    use crate::workspace::markdown::{Block, Inline, parse, plain};

    fn text(value: &str) -> Inline {
        Inline::Text(value.to_string())
    }

    #[test]
    fn headings_paragraphs_and_rules_are_read() {
        let blocks = parse("# Title\n\nSome words.\n\n---\n");
        assert_eq!(
            blocks,
            vec![
                Block::Heading {
                    level: 1,
                    content: vec![text("Title")]
                },
                Block::Paragraph(vec![text("Some words.")]),
                Block::Rule,
            ]
        );
    }

    #[test]
    fn bold_italic_and_inline_code_nest() {
        let blocks = parse("A **bold *and italic* word** and `code`.");
        let Block::Paragraph(content) = &blocks[0] else {
            panic!("expected a paragraph, got {blocks:?}");
        };
        assert_eq!(plain(content), "A bold and italic word and code.");
        assert!(matches!(content[1], Inline::Bold(_)));
        assert!(
            content
                .iter()
                .any(|inline| matches!(inline, Inline::Code(code) if code == "code"))
        );
    }

    #[test]
    fn links_keep_their_text_and_address() {
        let blocks = parse("See [the docs](https://example.com/docs).");
        let Block::Paragraph(content) = &blocks[0] else {
            panic!("expected a paragraph");
        };
        assert!(content.iter().any(|inline| matches!(
            inline,
            Inline::Link { url, .. } if url == "https://example.com/docs"
        )));
    }

    #[test]
    fn ordered_and_unordered_lists_keep_their_items() {
        let blocks = parse("1. first\n2. second\n\n- alpha\n- beta\n");
        assert!(matches!(&blocks[0], Block::List { ordered: true, items } if items.len() == 2));
        assert!(matches!(&blocks[1], Block::List { ordered: false, items } if items.len() == 2));
    }

    #[test]
    fn a_table_has_its_header_and_rows() {
        let blocks =
            parse("| Name | Due |\n| --- | --- |\n| Login | Friday |\n| Billing | Monday |\n");
        let Block::Table { header, rows } = &blocks[0] else {
            panic!("expected a table, got {blocks:?}");
        };
        assert_eq!(header.len(), 2);
        assert_eq!(rows.len(), 2);
        assert_eq!(plain(&rows[1][0]), "Billing");
    }

    #[test]
    fn a_fenced_block_gives_its_code_without_the_fence_lines() {
        let blocks = parse("```rust\nfn main() {}\n```\n");
        assert_eq!(
            blocks,
            vec![Block::Code {
                language: Some("rust".to_string()),
                text: "fn main() {}".to_string(),
            }]
        );
    }

    #[test]
    fn a_fence_with_no_language_is_still_a_code_block() {
        let blocks = parse("```\nplain text\n```");
        assert_eq!(
            blocks,
            vec![Block::Code {
                language: None,
                text: "plain text".to_string(),
            }]
        );
    }

    // A reply as it arrives: each prefix of a complete reply is a state the screen can be in.
    const REPLY: &str = "## Plan\n\nThe **login** fix:\n\n- check `verify_password`\n- add a test\n\n| File | Change |\n| --- | --- |\n| login.rs | refuse empty |\n\n```rust\nif password.is_empty() {\n    return None;\n}\n```\n\nSee [the notes](https://example.com).\n";

    #[test]
    fn an_unclosed_fence_during_streaming_is_code_to_the_end_and_does_not_break() {
        let cut = REPLY.find("```rust").unwrap() + "```rust\nif password.is_empty() {".len();
        let blocks = parse(&REPLY[..cut]);
        let Some(Block::Code { language, text }) = blocks.last() else {
            panic!("the open fence should be the last block, got {blocks:?}");
        };
        assert_eq!(language.as_deref(), Some("rust"));
        assert!(text.starts_with("if password.is_empty() {"), "{text:?}");
    }

    #[test]
    fn every_prefix_of_a_reply_parses_without_panicking() {
        let mut previous = 0;
        for cut in REPLY.char_indices().map(|(index, _)| index) {
            let blocks = parse(&REPLY[..cut]);
            // Blocks only ever grow as more of the reply arrives, apart from a block being completed in place.
            assert!(blocks.len() + 1 >= previous, "lost blocks at {cut}");
            previous = blocks.len();
        }
        assert_eq!(parse(REPLY).len(), 6);
    }

    #[test]
    fn half_written_emphasis_and_tables_stay_text() {
        let blocks = parse("**bo");
        let Block::Paragraph(content) = &blocks[0] else {
            panic!("expected a paragraph, got {blocks:?}");
        };
        assert_eq!(plain(content), "**bo");
        let partial = parse("| a | b |");
        assert!(matches!(partial[0], Block::Paragraph(_)), "{partial:?}");
    }

    #[test]
    fn an_empty_reply_has_no_blocks() {
        assert!(parse("").is_empty());
    }
}
