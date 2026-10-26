//! Cutting a source into chunks: the pieces the knowledge base stores and a search returns.
//!
//! A chunk is something a person would call one thing. Code is cut by symbol, so a function comes back whole. Docs
//! are cut by heading. An issue is one record. A commit is its message and the files it touched. Each chunk knows
//! where it came from, so an answer can point back at the file and the lines.

use looji_domain::{Symbol, SymbolKind};

/// Longest a code chunk is, in lines. A longer symbol is cut into windows of this size.
pub const MAX_CODE_LINES: usize = 100;
/// How many lines one window repeats from the one before, so a cut doesn't split an idea in half.
const WINDOW_OVERLAP: usize = 8;
/// Lines of a type taken as its own chunk when its methods are chunked separately.
const HEAD_LINES: usize = 30;
/// Code outside any symbol is chunked in windows of this many lines.
const LOOSE_WINDOW: usize = 60;
/// Fewest non-blank lines for loose code to be worth a chunk.
const LOOSE_MIN_LINES: usize = 3;
/// Longest a doc or issue chunk is, in characters, about 400 tokens.
pub const MAX_TEXT_CHARS: usize = 1600;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SourceKind {
    Code,
    Doc,
    Issue,
    Commit,
}

impl SourceKind {
    pub const ALL: [SourceKind; 4] = [
        SourceKind::Code,
        SourceKind::Doc,
        SourceKind::Issue,
        SourceKind::Commit,
    ];

    pub fn stored(self) -> &'static str {
        match self {
            SourceKind::Code => "code",
            SourceKind::Doc => "doc",
            SourceKind::Issue => "issue",
            SourceKind::Commit => "commit",
        }
    }

    pub fn from_stored(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.stored() == text)
    }
}

/// One searchable piece of the project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    pub kind: SourceKind,
    /// The path, issue id, or commit it belongs to. A chunk is replaced with the rest of its source.
    pub source: String,
    /// The file, for code and docs.
    pub path: Option<String>,
    /// The symbol, or the heading path, or the issue's title.
    pub label: Option<String>,
    /// 1-based, inclusive. Zero for chunks that aren't lines of a file.
    pub start_line: u32,
    pub end_line: u32,
    pub text: String,
}

/// An issue as the knowledge base sees it. The UI fills it from the task and its team.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueDoc {
    pub id: String,
    pub title: String,
    pub description: String,
    pub status: String,
    pub priority: Option<String>,
    pub assignee: Option<String>,
    pub tags: Vec<String>,
}

/// A commit, with the files it changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitDoc {
    pub sha: String,
    pub author: String,
    pub message: String,
    pub files: Vec<String>,
}

/// The chunks of a code file. Each symbol is a chunk. A symbol that holds other symbols, such as a class or an
/// impl block, is a chunk of its own opening lines, and its members are chunks of their own. Lines that belong to no
/// symbol are chunked in windows when there is enough in them to matter.
pub fn chunk_code(path: &str, source: &str, symbols: &[Symbol]) -> Vec<Chunk> {
    let lines: Vec<&str> = source.lines().collect();
    if lines.is_empty() {
        return Vec::new();
    }
    let mut ordered: Vec<&Symbol> = symbols
        .iter()
        .filter(|symbol| symbol.start_line >= 1 && symbol.end_line >= symbol.start_line)
        .collect();
    ordered.sort_by(|a, b| {
        a.start_line
            .cmp(&b.start_line)
            .then(b.end_line.cmp(&a.end_line))
    });

    let mut chunks = Vec::new();
    let mut covered = vec![false; lines.len() + 1];
    for symbol in &ordered {
        let start = symbol.start_line as usize;
        let end = (symbol.end_line as usize).min(lines.len());
        if start > end {
            continue;
        }
        // Where the first member starts, if this symbol holds any.
        let first_member = ordered
            .iter()
            .filter(|other| {
                other.start_line > symbol.start_line
                    && other.end_line <= symbol.end_line
                    && !std::ptr::eq(*other, symbol)
            })
            .map(|other| other.start_line as usize)
            .min();
        let own_end = match first_member {
            Some(member) => (member - 1).clamp(start, (start + HEAD_LINES - 1).min(end)),
            None => end,
        };
        let label = match symbol.kind {
            SymbolKind::Function => format!("fn {}", symbol.name),
            SymbolKind::Type => format!("type {}", symbol.name),
            SymbolKind::Module => format!("mod {}", symbol.name),
        };
        for (from, to) in windows(start, own_end) {
            chunks.push(Chunk {
                kind: SourceKind::Code,
                source: path.to_string(),
                path: Some(path.to_string()),
                label: Some(label.clone()),
                start_line: from as u32,
                end_line: to as u32,
                text: lines[from - 1..to].join("\n"),
            });
        }
    }
    // Every symbol's whole range counts as covered, so only code outside all symbols is chunked loosely.
    for symbol in &ordered {
        let end = (symbol.end_line as usize).min(lines.len());
        for flag in covered
            .iter_mut()
            .take(end + 1)
            .skip(symbol.start_line as usize)
        {
            *flag = true;
        }
    }

    // The lines no symbol covers, in runs.
    let mut line = 1;
    while line <= lines.len() {
        if covered[line] {
            line += 1;
            continue;
        }
        let run_start = line;
        while line <= lines.len() && !covered[line] {
            line += 1;
        }
        let run_end = line - 1;
        let busy = lines[run_start - 1..run_end]
            .iter()
            .filter(|text| !text.trim().is_empty())
            .count();
        if busy < LOOSE_MIN_LINES {
            continue;
        }
        let mut from = run_start;
        while from <= run_end {
            let to = (from + LOOSE_WINDOW - 1).min(run_end);
            chunks.push(Chunk {
                kind: SourceKind::Code,
                source: path.to_string(),
                path: Some(path.to_string()),
                label: None,
                start_line: from as u32,
                end_line: to as u32,
                text: lines[from - 1..to].join("\n"),
            });
            from = to + 1;
        }
    }
    chunks.sort_by_key(|chunk| chunk.start_line);
    chunks
}

/// A file with no symbols to cut at, such as one the parser couldn't read: windows of lines.
pub fn chunk_plain(path: &str, kind: SourceKind, source: &str) -> Vec<Chunk> {
    let lines: Vec<&str> = source.lines().collect();
    windows(1, lines.len())
        .into_iter()
        .filter(|(from, to)| {
            lines[from - 1..*to]
                .iter()
                .any(|line| !line.trim().is_empty())
        })
        .map(|(from, to)| Chunk {
            kind,
            source: path.to_string(),
            path: Some(path.to_string()),
            label: None,
            start_line: from as u32,
            end_line: to as u32,
            text: lines[from - 1..to].join("\n"),
        })
        .collect()
}

/// The 1-based inclusive line ranges that cover `start..=end` in windows of at most `MAX_CODE_LINES`, each
/// starting `WINDOW_OVERLAP` lines before the end of the one before. A range that fits is returned as it is.
fn windows(start: usize, end: usize) -> Vec<(usize, usize)> {
    if end < start {
        return Vec::new();
    }
    if end - start < MAX_CODE_LINES {
        return vec![(start, end)];
    }
    let mut out = Vec::new();
    let mut from = start;
    loop {
        let to = (from + MAX_CODE_LINES - 1).min(end);
        out.push((from, to));
        if to == end {
            break;
        }
        from = to + 1 - WINDOW_OVERLAP;
    }
    out
}

/// The chunks of a markdown document: one per heading section, with the heading path as its label, cut at
/// paragraphs when a section is long. Text before the first heading is its own chunk.
pub fn chunk_markdown(path: &str, source: &str) -> Vec<Chunk> {
    let mut chunks = Vec::new();
    let mut trail: Vec<(usize, String)> = Vec::new();
    let mut body: Vec<&str> = Vec::new();
    let mut section_start = 1usize;
    let mut in_fence = false;

    let flush = |trail: &[(usize, String)],
                 body: &mut Vec<&str>,
                 start: usize,
                 end: usize,
                 chunks: &mut Vec<Chunk>| {
        let text = body.join("\n");
        body.clear();
        if text.trim().is_empty() {
            return;
        }
        let label = (!trail.is_empty()).then(|| {
            trail
                .iter()
                .map(|(_, title)| title.as_str())
                .collect::<Vec<_>>()
                .join(" \u{203a} ")
        });
        for piece in split_paragraphs(text.trim(), MAX_TEXT_CHARS) {
            chunks.push(Chunk {
                kind: SourceKind::Doc,
                source: path.to_string(),
                path: Some(path.to_string()),
                label: label.clone(),
                start_line: start as u32,
                end_line: end as u32,
                text: piece,
            });
        }
    };

    let total = source.lines().count();
    for (index, line) in source.lines().enumerate() {
        let number = index + 1;
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
        }
        let heading = (!in_fence).then(|| heading_of(line)).flatten();
        if let Some((level, title)) = heading {
            flush(
                &trail,
                &mut body,
                section_start,
                number.saturating_sub(1).max(section_start),
                &mut chunks,
            );
            while trail.last().is_some_and(|(deeper, _)| *deeper >= level) {
                trail.pop();
            }
            trail.push((level, title));
            section_start = number;
            body.push(line);
        } else {
            body.push(line);
        }
    }
    flush(
        &trail,
        &mut body,
        section_start,
        total.max(section_start),
        &mut chunks,
    );
    chunks
}

/// The level and title of a markdown heading line, if it is one.
fn heading_of(line: &str) -> Option<(usize, String)> {
    let trimmed = line.trim_start();
    let level = trimmed.chars().take_while(|c| *c == '#').count();
    if level == 0 || level > 6 {
        return None;
    }
    let rest = trimmed[level..].strip_prefix(' ')?;
    let title = rest.trim().trim_end_matches('#').trim();
    (!title.is_empty()).then(|| (level, title.to_string()))
}

/// Cuts `text` at blank lines into pieces of at most `limit` characters, joining short paragraphs. A paragraph
/// longer than the limit is cut at the last whitespace before it.
fn split_paragraphs(text: &str, limit: usize) -> Vec<String> {
    let mut pieces: Vec<String> = Vec::new();
    let mut current = String::new();
    for paragraph in text.split("\n\n") {
        let paragraph = paragraph.trim_end();
        if paragraph.is_empty() {
            continue;
        }
        if !current.is_empty() && current.len() + 2 + paragraph.len() > limit {
            pieces.push(std::mem::take(&mut current));
        }
        if paragraph.len() > limit {
            let mut rest = paragraph;
            while rest.len() > limit {
                let mut cut = limit;
                while !rest.is_char_boundary(cut) {
                    cut -= 1;
                }
                let at = rest[..cut]
                    .rfind(char::is_whitespace)
                    .filter(|at| *at > 0)
                    .unwrap_or(cut);
                pieces.push(rest[..at].trim_end().to_string());
                rest = rest[at..].trim_start();
            }
            current = rest.to_string();
            continue;
        }
        if !current.is_empty() {
            current.push_str("\n\n");
        }
        current.push_str(paragraph);
    }
    if !current.is_empty() {
        pieces.push(current);
    }
    pieces
}

/// An issue as chunks: the title and fields lead every chunk, so a piece of a long description still says which
/// issue it is from.
pub fn chunk_issue(issue: &IssueDoc) -> Vec<Chunk> {
    let mut facts = vec![format!("Status: {}", issue.status)];
    if let Some(priority) = &issue.priority {
        facts.push(format!("Priority: {priority}"));
    }
    if let Some(assignee) = &issue.assignee {
        facts.push(format!("Assignee: {assignee}"));
    }
    if !issue.tags.is_empty() {
        facts.push(format!("Tags: {}", issue.tags.join(", ")));
    }
    let head = format!("{}\n{}", issue.title.trim(), facts.join(" \u{b7} "));
    let description = issue.description.trim();
    let pieces = if description.is_empty() {
        vec![String::new()]
    } else {
        split_paragraphs(
            description,
            MAX_TEXT_CHARS - head.len().min(MAX_TEXT_CHARS / 2),
        )
    };
    pieces
        .into_iter()
        .map(|piece| Chunk {
            kind: SourceKind::Issue,
            source: issue.id.clone(),
            path: None,
            label: Some(issue.title.trim().to_string()),
            start_line: 0,
            end_line: 0,
            text: if piece.is_empty() {
                head.clone()
            } else {
                format!("{head}\n\n{piece}")
            },
        })
        .collect()
}

/// A commit as one chunk: its message, then the files it touched.
pub fn chunk_commit(commit: &CommitDoc) -> Chunk {
    let mut text = commit.message.trim().to_string();
    if !commit.files.is_empty() {
        text.push_str("\n\nFiles: ");
        text.push_str(&commit.files.join(", "));
    }
    Chunk {
        kind: SourceKind::Commit,
        source: commit.sha.clone(),
        path: None,
        label: Some(
            commit
                .message
                .lines()
                .next()
                .unwrap_or_default()
                .trim()
                .to_string(),
        ),
        start_line: 0,
        end_line: 0,
        text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn symbol(name: &str, kind: SymbolKind, start: u32, end: u32) -> Symbol {
        Symbol {
            name: name.to_string(),
            kind,
            path: "src/lib.rs".to_string(),
            start_line: start,
            end_line: end,
        }
    }

    fn numbered(count: usize) -> String {
        (1..=count)
            .map(|n| format!("line {n}"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn a_function_is_one_chunk_with_its_lines() {
        let source = numbered(8);
        let chunks = chunk_code(
            "src/lib.rs",
            &source,
            &[symbol("login", SymbolKind::Function, 3, 8)],
        );
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].start_line, 3);
        assert_eq!(chunks[0].end_line, 8);
        assert_eq!(chunks[0].label.as_deref(), Some("fn login"));
        assert!(chunks[0].text.starts_with("line 3") && chunks[0].text.ends_with("line 8"));
    }

    #[test]
    fn a_long_symbol_is_cut_into_overlapping_windows_that_cover_it() {
        let source = numbered(250);
        let chunks = chunk_code(
            "a.rs",
            &source,
            &[symbol("big", SymbolKind::Function, 1, 250)],
        );
        assert!(chunks.len() >= 3);
        assert!(
            chunks
                .iter()
                .all(|c| (c.end_line - c.start_line + 1) as usize <= MAX_CODE_LINES)
        );
        assert_eq!(chunks.first().unwrap().start_line, 1);
        assert_eq!(chunks.last().unwrap().end_line, 250);
        for pair in chunks.windows(2) {
            assert!(
                pair[1].start_line <= pair[0].end_line,
                "windows overlap, so no line is lost"
            );
        }
    }

    #[test]
    fn a_type_with_methods_is_its_opening_lines_and_each_method() {
        let source = numbered(40);
        let symbols = [
            symbol("Session", SymbolKind::Type, 1, 40),
            symbol("new", SymbolKind::Function, 5, 12),
            symbol("close", SymbolKind::Function, 20, 30),
        ];
        let chunks = chunk_code("a.rs", &source, &symbols);
        let labels: Vec<_> = chunks.iter().filter_map(|c| c.label.as_deref()).collect();
        assert_eq!(labels, ["type Session", "fn new", "fn close"]);
        // The type's own chunk stops before its first method.
        assert_eq!(chunks[0].end_line, 4);
    }

    #[test]
    fn loose_code_between_symbols_is_kept_when_there_is_enough_of_it() {
        let source = "use a;\nuse b;\nuse c;\n\nfn one() {\n}\n\nfn two() {\n}\n";
        let symbols = [
            symbol("one", SymbolKind::Function, 5, 6),
            symbol("two", SymbolKind::Function, 8, 9),
        ];
        let chunks = chunk_code("a.rs", source, &symbols);
        assert!(
            chunks
                .iter()
                .any(|c| c.label.is_none() && c.text.contains("use a;"))
        );
    }

    #[test]
    fn a_file_with_no_symbols_is_chunked_in_windows() {
        let chunks = chunk_plain("notes.txt", SourceKind::Doc, &numbered(130));
        assert_eq!(chunks.first().unwrap().start_line, 1);
        assert_eq!(chunks.last().unwrap().end_line, 130);
    }

    #[test]
    fn markdown_is_cut_by_heading_with_the_heading_path_as_its_label() {
        let doc = "# Guide\nIntro.\n\n## Install\n\n### Linux\nUse apt.\n\n### Windows\nUse the installer.\n\n## Usage\nRun it.\n";
        let chunks = chunk_markdown("README.md", doc);
        let labels: Vec<_> = chunks.iter().filter_map(|c| c.label.clone()).collect();
        assert_eq!(
            labels,
            [
                "Guide",
                "Guide \u{203a} Install",
                "Guide \u{203a} Install \u{203a} Linux",
                "Guide \u{203a} Install \u{203a} Windows",
                "Guide \u{203a} Usage"
            ]
        );
        assert!(chunks.iter().any(|c| c.text.contains("Use apt.")));
    }

    #[test]
    fn a_hash_inside_a_code_fence_is_not_a_heading() {
        let doc = "# Title\n```\n# not a heading\n```\n";
        let chunks = chunk_markdown("a.md", doc);
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].text.contains("# not a heading"));
    }

    #[test]
    fn a_long_section_is_cut_at_paragraphs() {
        let paragraph = "word ".repeat(200);
        let doc = format!("# Long\n{paragraph}\n\n{paragraph}\n\n{paragraph}\n");
        let chunks = chunk_markdown("a.md", &doc);
        assert!(chunks.len() >= 2);
        assert!(chunks.iter().all(|c| c.text.len() <= MAX_TEXT_CHARS));
    }

    #[test]
    fn an_issue_chunk_leads_with_the_title_and_the_facts() {
        let issue = IssueDoc {
            id: "t1".into(),
            title: "Fix login timeout".into(),
            description: "Users are signed out after a minute.".into(),
            status: "In progress".into(),
            priority: Some("High".into()),
            assignee: Some("Maya".into()),
            tags: vec!["auth".into()],
        };
        let chunks = chunk_issue(&issue);
        assert_eq!(chunks.len(), 1);
        assert!(
            chunks[0]
                .text
                .starts_with("Fix login timeout\nStatus: In progress")
        );
        assert!(chunks[0].text.contains("Tags: auth"));
        assert!(chunks[0].text.contains("signed out"));
    }

    #[test]
    fn a_long_issue_description_keeps_the_title_on_every_piece() {
        let issue = IssueDoc {
            id: "t2".into(),
            title: "Rework billing".into(),
            description: format!("{}\n\n{}", "alpha ".repeat(300), "beta ".repeat(300)),
            status: "Todo".into(),
            priority: None,
            assignee: None,
            tags: Vec::new(),
        };
        let chunks = chunk_issue(&issue);
        assert!(chunks.len() >= 2);
        assert!(chunks.iter().all(|c| c.text.starts_with("Rework billing")));
    }

    #[test]
    fn a_commit_is_its_message_and_its_files() {
        let chunk = chunk_commit(&CommitDoc {
            sha: "abc".into(),
            author: "Maya".into(),
            message: "Add retry to the uploader\n\nIt gave up after one failure.".into(),
            files: vec!["src/upload.rs".into(), "src/retry.rs".into()],
        });
        assert_eq!(chunk.label.as_deref(), Some("Add retry to the uploader"));
        assert!(chunk.text.ends_with("Files: src/upload.rs, src/retry.rs"));
    }
}
