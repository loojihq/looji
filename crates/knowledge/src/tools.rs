//! The tools an AI agent uses to explore a project: search, read a file, find a definition, find uses, list and read
//! issues, see recent changes, and get a map of the repository. They only read. Each one answers in text the model can
//! use, and names the sources it drew on, so an answer can cite them.
//!
//! The same tools serve both providers. The Anthropic API calls them from the agent loop, and Claude Code calls them
//! through the MCP server in `mcp`.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
};

use serde_json::{Value, json};

use crate::{
    Error, Result, SourceKind,
    chunk::IssueDoc,
    facts::{IssueFilter, contains_word},
    safety,
    store::{Hit, KnowledgeStore, SearchQuery},
};

/// Most characters one tool answer may hold. Longer answers are cut and say so, so one call can't fill the context.
const MAX_ANSWER_CHARS: usize = 14_000;
/// Most lines of a file one read returns.
const MAX_READ_LINES: usize = 300;
/// Largest file `read_file` opens.
const MAX_READ_BYTES: u64 = 2 * 1024 * 1024;
/// Lines of a chunk shown in a search result.
const SNIPPET_LINES: usize = 24;

/// A tool the model can call.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolDef {
    pub name: &'static str,
    pub description: &'static str,
    /// A JSON Schema for the arguments.
    pub schema: Value,
}

/// Where part of an answer came from, for the "Sources used" list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolSource {
    pub kind: SourceKind,
    /// The file path, issue id, or commit id.
    pub reference: String,
    /// What to call it: the symbol, the issue's title, or the commit's subject.
    pub label: String,
    pub start_line: u32,
    pub end_line: u32,
}

/// What a tool call gave back.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ToolOutput {
    pub text: String,
    pub sources: Vec<ToolSource>,
    /// The call failed, and `text` says why, so the model can try something else.
    pub is_error: bool,
}

impl ToolOutput {
    fn ok(text: String, sources: Vec<ToolSource>) -> Self {
        Self {
            text: cut(text),
            sources,
            is_error: false,
        }
    }

    fn error(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            sources: Vec::new(),
            is_error: true,
        }
    }
}

fn cut(mut text: String) -> String {
    if text.len() > MAX_ANSWER_CHARS {
        let mut end = MAX_ANSWER_CHARS;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
        text.push_str("\n[cut: the answer was too long. Ask for less, or narrow the question.]");
    }
    text
}

/// The tools, in the order they are offered.
pub fn definitions() -> Vec<ToolDef> {
    let object = |properties: Value, required: &[&str]| json!({ "type": "object", "properties": properties, "required": required });
    vec![
        ToolDef {
            name: "search",
            description: "Search the project's knowledge: code, docs, issues, and commits. Use words from the question and from likely names. Returns the best matching passages with where they are. Try different words if the first search finds little.",
            schema: object(
                json!({
                    "query": { "type": "string", "description": "What to look for, in words or identifiers." },
                    "kinds": { "type": "array", "items": { "type": "string", "enum": ["code", "doc", "issue", "commit"] }, "description": "Limit the search to these kinds." },
                    "limit": { "type": "integer", "description": "How many results, 1 to 15. Default 8." }
                }),
                &["query"],
            ),
        },
        ToolDef {
            name: "read_file",
            description: "Read lines of a file in the repository. Use it to see the code around a search result. At most 300 lines at a time.",
            schema: object(
                json!({
                    "path": { "type": "string", "description": "The file, relative to the repository root." },
                    "start_line": { "type": "integer", "description": "First line, from 1. Default 1." },
                    "end_line": { "type": "integer", "description": "Last line. Default 200 lines after the first." }
                }),
                &["path"],
            ),
        },
        ToolDef {
            name: "find_symbol",
            description: "Find where a function, type, or module is defined, by all or part of its name.",
            schema: object(json!({ "name": { "type": "string" } }), &["name"]),
        },
        ToolDef {
            name: "find_references",
            description: "Find the code and docs that use a name, as whole words. This is a text search, so it finds uses by name and cannot tell apart two things that share one.",
            schema: object(
                json!({
                    "name": { "type": "string" },
                    "limit": { "type": "integer", "description": "How many passages, up to 40. Default 20." }
                }),
                &["name"],
            ),
        },
        ToolDef {
            name: "list_issues",
            description: "List the project's issues, optionally filtered. Statuses are Backlog, To do, In progress, In review, and Done.",
            schema: object(
                json!({
                    "status": { "type": "string" },
                    "assignee": { "type": "string", "description": "Part of the assignee's name." },
                    "tag": { "type": "string" },
                    "text": { "type": "string", "description": "A word in the title or description." },
                    "limit": { "type": "integer", "description": "Default 25." }
                }),
                &[],
            ),
        },
        ToolDef {
            name: "get_issue",
            description: "Read one issue in full, with the commits that relate to it. Use the id from list_issues or a search result.",
            schema: object(
                json!({ "id": { "type": "string", "description": "The issue id, or the start of it." } }),
                &["id"],
            ),
        },
        ToolDef {
            name: "recent_changes",
            description: "The latest commits, newest first, optionally only those that touched a path.",
            schema: object(
                json!({
                    "path": { "type": "string", "description": "Part of a file path." },
                    "limit": { "type": "integer", "description": "Default 15." }
                }),
                &[],
            ),
        },
        ToolDef {
            name: "repo_map",
            description: "A map of the repository: its folders, how many files each holds, and in which languages. Start here to learn where things are.",
            schema: object(json!({}), &[]),
        },
    ]
}

/// The tools, bound to one project's knowledge and one repository.
pub struct KnowledgeTools {
    store: Mutex<KnowledgeStore>,
    project: String,
    root: PathBuf,
    embedder: Option<std::sync::Arc<dyn crate::vectors::Embedder>>,
}

impl KnowledgeTools {
    pub fn open(database: &Path, project: &str, root: &Path) -> Result<Self> {
        Ok(Self::with_store(
            KnowledgeStore::open(database)?,
            project,
            root,
        ))
    }

    pub fn with_store(store: KnowledgeStore, project: &str, root: &Path) -> Self {
        Self {
            store: Mutex::new(store),
            project: project.to_string(),
            root: root.to_path_buf(),
            embedder: None,
        }
    }

    /// Lets searches match by meaning as well as by words, once the project has vectors.
    pub fn with_embedder(mut self, embedder: std::sync::Arc<dyn crate::vectors::Embedder>) -> Self {
        self.embedder = Some(embedder);
        self
    }

    /// Runs one tool. A bad call is an answer with `is_error` set, never a failure of the caller.
    pub fn call(&self, name: &str, input: &Value) -> ToolOutput {
        let Ok(store) = self.store.lock() else {
            return ToolOutput::error("The knowledge base is busy. Try again.");
        };
        let outcome = match name {
            "search" => self.search(&store, input),
            "read_file" => self.read_file(input),
            "find_symbol" => self.find_symbol(&store, input),
            "find_references" => self.find_references(&store, input),
            "list_issues" => self.list_issues(&store, input),
            "get_issue" => self.get_issue(&store, input),
            "recent_changes" => self.recent_changes(&store, input),
            "repo_map" => self.repo_map(&store),
            other => return ToolOutput::error(format!("There is no tool called {other}.")),
        };
        outcome.unwrap_or_else(|err| ToolOutput::error(err.to_string()))
    }

    fn search(&self, store: &KnowledgeStore, input: &Value) -> Result<ToolOutput> {
        let query = text_arg(input, "query")?;
        let kinds: Vec<SourceKind> = input["kinds"]
            .as_array()
            .map(|list| {
                list.iter()
                    .filter_map(Value::as_str)
                    .filter_map(SourceKind::from_stored)
                    .collect()
            })
            .unwrap_or_default();
        let limit = number_arg(input, "limit", 8).clamp(1, 15);
        let request = SearchQuery {
            text: query.clone(),
            kinds,
            limit,
            ..Default::default()
        };
        let vector = self.embedder.as_ref().and_then(|embedder| {
            crate::vectors::embed_query(store, &self.project, embedder.as_ref(), &query)
                .map(|vector| (embedder.model_id().to_string(), vector))
        });
        let hits = store.search_hybrid(
            &self.project,
            &request,
            vector
                .as_ref()
                .map(|(model, vector)| (model.as_str(), vector.as_slice())),
        )?;
        if hits.is_empty() {
            return Ok(ToolOutput::ok(
                format!(
                    "Nothing matched \"{query}\". Try other words, or a name the code might use."
                ),
                Vec::new(),
            ));
        }
        let mut text = String::new();
        let mut sources = Vec::new();
        for (index, hit) in hits.iter().enumerate() {
            text.push_str(&format!(
                "[{}] {}\n{}\n\n",
                index + 1,
                describe(hit),
                snippet(hit)
            ));
            sources.push(source_of(hit));
        }
        Ok(ToolOutput::ok(text, sources))
    }

    fn read_file(&self, input: &Value) -> Result<ToolOutput> {
        let relative = text_arg(input, "path")?;
        let full = match self.resolve(&relative) {
            Ok(full) => full,
            Err(message) => return Ok(ToolOutput::error(message)),
        };
        let bytes = fs::read(&full).map_err(|err| Error::Io(err.to_string()))?;
        if !safety::looks_like_text(&bytes) {
            return Ok(ToolOutput::error("That file is not text."));
        }
        let text = String::from_utf8_lossy(&bytes);
        let (text, _) =
            safety::redact(&text).unwrap_or_else(|| ("[a private key was here]".to_string(), true));
        let lines: Vec<&str> = text.lines().collect();
        let start = number_arg(input, "start_line", 1).max(1);
        let wanted_end = number_arg(input, "end_line", start + 199).max(start);
        let end = wanted_end.min(start + MAX_READ_LINES - 1).min(lines.len());
        if start > lines.len() {
            return Ok(ToolOutput::error(format!(
                "{relative} has {} lines, so line {start} is past its end.",
                lines.len()
            )));
        }
        let mut out = format!("{relative}, lines {start} to {end} of {}:\n", lines.len());
        for number in start..=end {
            out.push_str(&format!("{number:>5} | {}\n", lines[number - 1]));
        }
        if end < wanted_end.min(lines.len()) {
            out.push_str(
                "[stopped at the most lines one read gives. Read on from the next line.]\n",
            );
        }
        Ok(ToolOutput::ok(
            out,
            vec![ToolSource {
                kind: if safety::classify(&relative) == safety::FileClass::Doc {
                    SourceKind::Doc
                } else {
                    SourceKind::Code
                },
                reference: relative.clone(),
                label: relative,
                start_line: start as u32,
                end_line: end as u32,
            }],
        ))
    }

    /// The file under the repository root that `relative` names, or why it can't be read.
    fn resolve(&self, relative: &str) -> std::result::Result<PathBuf, String> {
        let relative = relative.trim().replace('\\', "/");
        if relative.is_empty()
            || relative.starts_with('/')
            || relative.contains(':')
            || relative.split('/').any(|part| part == "..")
        {
            return Err(
                "The path has to be inside the repository, such as src/main.rs.".to_string(),
            );
        }
        if safety::is_secret_path(&relative) {
            return Err(
                "That file is kept out of reach because its name says it holds a secret."
                    .to_string(),
            );
        }
        let root = fs::canonicalize(&self.root)
            .map_err(|_| "The repository is not reachable.".to_string())?;
        let full = fs::canonicalize(root.join(&relative))
            .map_err(|_| format!("There is no file {relative}."))?;
        if !full.starts_with(&root) {
            return Err("The path has to be inside the repository.".to_string());
        }
        let metadata = fs::metadata(&full).map_err(|err| err.to_string())?;
        if !metadata.is_file() {
            return Err(format!("{relative} is not a file."));
        }
        if metadata.len() > MAX_READ_BYTES {
            return Err(format!("{relative} is too large to read."));
        }
        Ok(full)
    }

    fn find_symbol(&self, store: &KnowledgeStore, input: &Value) -> Result<ToolOutput> {
        let name = text_arg(input, "name")?;
        let hits = store.find_labelled(&self.project, &name, 10)?;
        if hits.is_empty() {
            return Ok(ToolOutput::ok(
                format!("No definition has a name containing \"{name}\"."),
                Vec::new(),
            ));
        }
        let mut text = String::new();
        let mut sources = Vec::new();
        for hit in &hits {
            let first = hit.text.lines().next().unwrap_or_default().trim();
            text.push_str(&format!("{}\n    {first}\n", describe(hit)));
            sources.push(source_of(hit));
        }
        Ok(ToolOutput::ok(text, sources))
    }

    fn find_references(&self, store: &KnowledgeStore, input: &Value) -> Result<ToolOutput> {
        let name = text_arg(input, "name")?;
        let limit = number_arg(input, "limit", 20).clamp(1, 40);
        let hits = store.find_text(&self.project, &name, limit)?;
        if hits.is_empty() {
            return Ok(ToolOutput::ok(
                format!("\"{name}\" is not used as a whole word anywhere that is indexed."),
                Vec::new(),
            ));
        }
        let mut text = format!("Uses of \"{name}\", found by text:\n");
        let mut sources = Vec::new();
        for hit in &hits {
            let path = hit.path.clone().unwrap_or_default();
            for (offset, line) in hit.text.lines().enumerate() {
                if contains_word(line, &name) {
                    text.push_str(&format!(
                        "{path}:{}  {}\n",
                        hit.start_line as usize + offset,
                        line.trim()
                    ));
                }
            }
            sources.push(source_of(hit));
        }
        Ok(ToolOutput::ok(text, sources))
    }

    fn list_issues(&self, store: &KnowledgeStore, input: &Value) -> Result<ToolOutput> {
        let filter = IssueFilter {
            status: optional_text(input, "status"),
            assignee: optional_text(input, "assignee"),
            tag: optional_text(input, "tag"),
            text: optional_text(input, "text"),
            limit: number_arg(input, "limit", 25).clamp(1, 100),
        };
        let issues = store.issues(&self.project, &filter)?;
        if issues.is_empty() {
            return Ok(ToolOutput::ok("No issue matches.".to_string(), Vec::new()));
        }
        let mut text = String::new();
        let mut sources = Vec::new();
        for issue in &issues {
            text.push_str(&format!("{}\n", issue_line(issue)));
            sources.push(issue_source(issue));
        }
        Ok(ToolOutput::ok(text, sources))
    }

    fn get_issue(&self, store: &KnowledgeStore, input: &Value) -> Result<ToolOutput> {
        let id = text_arg(input, "id")?;
        let issue = match store.issue(&self.project, &id)? {
            Some(issue) => Some(issue),
            None => {
                let matches: Vec<IssueDoc> = store
                    .issues(&self.project, &IssueFilter::default())?
                    .into_iter()
                    .filter(|issue| issue.id.starts_with(&id))
                    .collect();
                (matches.len() == 1).then(|| matches[0].clone())
            }
        };
        let Some(issue) = issue else {
            return Ok(ToolOutput::error(format!(
                "No issue has the id {id}. List the issues to find it."
            )));
        };
        let mut text = format!("{}\n\n{}\n", issue_line(&issue), issue.description.trim());
        let mut sources = vec![issue_source(&issue)];
        let commits = store.commits_for_issue(&self.project, &issue.id)?;
        if !commits.is_empty() {
            text.push_str("\nCommits that relate to it:\n");
            for sha in commits {
                if let Some(commit) = store.commit(&self.project, &sha)? {
                    let subject = commit.message.lines().next().unwrap_or_default();
                    text.push_str(&format!(
                        "{} {} ({})\n",
                        short(&commit.sha),
                        subject,
                        commit.files.join(", ")
                    ));
                    sources.push(ToolSource {
                        kind: SourceKind::Commit,
                        reference: commit.sha.clone(),
                        label: subject.to_string(),
                        start_line: 0,
                        end_line: 0,
                    });
                }
            }
        }
        Ok(ToolOutput::ok(text, sources))
    }

    fn recent_changes(&self, store: &KnowledgeStore, input: &Value) -> Result<ToolOutput> {
        let path = optional_text(input, "path");
        let limit = number_arg(input, "limit", 15).clamp(1, 50);
        let commits = store.commits(&self.project, path.as_deref(), limit)?;
        if commits.is_empty() {
            return Ok(ToolOutput::ok(
                "No commits are known for that.".to_string(),
                Vec::new(),
            ));
        }
        let mut text = String::new();
        let mut sources = Vec::new();
        for commit in &commits {
            let subject = commit.message.lines().next().unwrap_or_default();
            let files = if commit.files.len() > 6 {
                format!(
                    "{} and {} more",
                    commit.files[..6].join(", "),
                    commit.files.len() - 6
                )
            } else {
                commit.files.join(", ")
            };
            text.push_str(&format!(
                "{} {} {}: {} ({files})\n",
                short(&commit.sha),
                day(commit.date),
                commit.author,
                subject
            ));
            sources.push(ToolSource {
                kind: SourceKind::Commit,
                reference: commit.sha.clone(),
                label: subject.to_string(),
                start_line: 0,
                end_line: 0,
            });
        }
        Ok(ToolOutput::ok(text, sources))
    }

    fn repo_map(&self, store: &KnowledgeStore) -> Result<ToolOutput> {
        let files = store.files(&self.project)?;
        if files.is_empty() {
            return Ok(ToolOutput::ok(
                "Nothing in the repository has been indexed yet.".to_string(),
                Vec::new(),
            ));
        }
        Ok(ToolOutput::ok(render_map(&files), Vec::new()))
    }
}

/// The repository map as text: each folder to two levels, with its file count and languages.
pub fn render_map(files: &[(String, SourceKind, u64)]) -> String {
    #[derive(Default)]
    struct Folder {
        files: usize,
        languages: BTreeMap<&'static str, usize>,
    }
    let language = |path: &str| -> &'static str {
        match path
            .rsplit_once('.')
            .map(|(_, extension)| extension.to_lowercase())
            .as_deref()
        {
            Some("rs") => "Rust",
            Some("py") => "Python",
            Some("js" | "mjs" | "cjs" | "jsx") => "JavaScript",
            Some("ts" | "mts" | "cts") => "TypeScript",
            Some("tsx") => "TSX",
            Some("go") => "Go",
            Some("md" | "mdx" | "markdown" | "rst" | "txt") => "docs",
            _ => "other",
        }
    };
    let mut folders: BTreeMap<String, Folder> = BTreeMap::new();
    for (path, _, _) in files {
        let parts: Vec<&str> = path.split('/').collect();
        let keys = match parts.len() {
            1 => vec![".".to_string()],
            2 => vec![format!("{}/", parts[0])],
            _ => vec![
                format!("{}/", parts[0]),
                format!("{}/{}/", parts[0], parts[1]),
            ],
        };
        for key in keys {
            let folder = folders.entry(key).or_default();
            folder.files += 1;
            *folder.languages.entry(language(path)).or_insert(0) += 1;
        }
    }
    let mut out = format!("Repository map ({} indexed files):\n", files.len());
    for (name, folder) in &folders {
        let depth = name.trim_end_matches('/').matches('/').count();
        let languages: Vec<String> = folder
            .languages
            .iter()
            .map(|(language, count)| format!("{language} {count}"))
            .collect();
        out.push_str(&format!(
            "{}{} \u{2014} {} files ({})\n",
            "  ".repeat(depth),
            name,
            folder.files,
            languages.join(", ")
        ));
    }
    out
}

fn text_arg(input: &Value, key: &str) -> Result<String> {
    optional_text(input, key).ok_or_else(|| Error::Io(format!("The {key} argument is needed.")))
}

fn optional_text(input: &Value, key: &str) -> Option<String> {
    input[key]
        .as_str()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

fn number_arg(input: &Value, key: &str, default: usize) -> usize {
    input[key]
        .as_u64()
        .map(|number| number as usize)
        .unwrap_or(default)
}

fn short(sha: &str) -> &str {
    &sha[..sha.len().min(8)]
}

/// A day, as `2026-10-26`, from seconds since the epoch.
fn day(seconds: i64) -> String {
    if seconds <= 0 {
        return "unknown date".to_string();
    }
    let days = seconds.div_euclid(86_400);
    // The civil date for a day count, after Howard Hinnant's algorithm.
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_part = (5 * day_of_year + 2) / 153;
    let day_of_month = day_of_year - (153 * month_part + 2) / 5 + 1;
    let month = if month_part < 10 {
        month_part + 3
    } else {
        month_part - 9
    };
    let year = if month <= 2 { year + 1 } else { year };
    format!("{year:04}-{month:02}-{day_of_month:02}")
}

fn describe(hit: &Hit) -> String {
    match hit.kind {
        SourceKind::Code | SourceKind::Doc => format!(
            "{}:{}-{}{}",
            hit.path.as_deref().unwrap_or("?"),
            hit.start_line,
            hit.end_line,
            hit.label
                .as_ref()
                .map(|label| format!("  ({label})"))
                .unwrap_or_default()
        ),
        SourceKind::Issue => format!(
            "issue {} \"{}\"",
            hit.source,
            hit.label.as_deref().unwrap_or_default()
        ),
        SourceKind::Commit => format!(
            "commit {} \"{}\"",
            short(&hit.source),
            hit.label.as_deref().unwrap_or_default()
        ),
    }
}

fn snippet(hit: &Hit) -> String {
    let lines: Vec<&str> = hit.text.lines().collect();
    let mut shown = lines
        .iter()
        .take(SNIPPET_LINES)
        .copied()
        .collect::<Vec<_>>()
        .join("\n");
    if lines.len() > SNIPPET_LINES {
        shown.push_str(&format!(
            "\n... ({} more lines)",
            lines.len() - SNIPPET_LINES
        ));
    }
    shown
}

fn source_of(hit: &Hit) -> ToolSource {
    ToolSource {
        kind: hit.kind,
        reference: match hit.kind {
            SourceKind::Code | SourceKind::Doc => hit.path.clone().unwrap_or_default(),
            _ => hit.source.clone(),
        },
        label: hit.label.clone().unwrap_or_default(),
        start_line: hit.start_line,
        end_line: hit.end_line,
    }
}

fn issue_line(issue: &IssueDoc) -> String {
    let mut line = format!("{} [{}] {}", issue.id, issue.status, issue.title);
    if let Some(priority) = &issue.priority {
        line.push_str(&format!(" \u{b7} {priority} priority"));
    }
    if let Some(assignee) = &issue.assignee {
        line.push_str(&format!(" \u{b7} {assignee}"));
    }
    if !issue.tags.is_empty() {
        line.push_str(&format!(" \u{b7} {}", issue.tags.join(", ")));
    }
    line
}

fn issue_source(issue: &IssueDoc) -> ToolSource {
    ToolSource {
        kind: SourceKind::Issue,
        reference: issue.id.clone(),
        label: issue.title.clone(),
        start_line: 0,
        end_line: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{chunk::CommitDoc, sync::Syncer};
    use tempfile::TempDir;

    fn write(root: &Path, path: &str, text: &str) {
        let full = root.join(path);
        fs::create_dir_all(full.parent().unwrap()).unwrap();
        fs::write(full, text).unwrap();
    }

    fn issue(id: &str, title: &str, status: &str) -> IssueDoc {
        IssueDoc {
            id: id.into(),
            title: title.into(),
            description: format!("{title} in detail."),
            status: status.into(),
            priority: Some("High".into()),
            assignee: Some("Maya".into()),
            tags: vec!["auth".into()],
        }
    }

    fn setup() -> (TempDir, KnowledgeTools) {
        let dir = TempDir::new().unwrap();
        write(
            dir.path(),
            "src/auth.rs",
            "pub fn verify_password(input: &str) -> bool {\n    input.len() > 8\n}\n\npub fn sign_in(p: &str) -> bool {\n    verify_password(p)\n}\n",
        );
        write(
            dir.path(),
            "src/pay.rs",
            "pub fn charge_card(cents: u64) -> u64 {\n    cents\n}\n",
        );
        write(
            dir.path(),
            "docs/guide.md",
            "# Guide\n\n## Login\nUsers sign in with a password.\n",
        );
        write(dir.path(), ".env", "SECRET=1\n");
        let mut store = KnowledgeStore::open_in_memory().unwrap();
        let mut syncer = Syncer::new().unwrap();
        syncer.sync_repo(&mut store, "p", dir.path()).unwrap();
        syncer
            .sync_issues(
                &mut store,
                "p",
                &[
                    issue("11111111-aaaa", "Fix login timeout", "In progress"),
                    issue("22222222-bbbb", "Add refunds", "Todo"),
                ],
            )
            .unwrap();
        let commit = CommitDoc {
            sha: "abcdef123456".into(),
            author: "Maya".into(),
            date: 1_761_436_800,
            message: "Fix the login timeout\n\nIdle sessions lasted too long.".into(),
            files: vec!["src/auth.rs".into()],
        };
        syncer
            .sync_commits(&mut store, "p", std::slice::from_ref(&commit))
            .unwrap();
        store
            .link_commit("p", "abcdef123456", "11111111-aaaa")
            .unwrap();
        let tools = KnowledgeTools::with_store(store, "p", dir.path());
        (dir, tools)
    }

    fn call(tools: &KnowledgeTools, name: &str, input: Value) -> ToolOutput {
        tools.call(name, &input)
    }

    #[test]
    fn every_tool_has_a_name_a_description_and_an_object_schema() {
        let tools = definitions();
        assert_eq!(tools.len(), 8);
        for tool in &tools {
            assert!(!tool.description.is_empty());
            assert_eq!(tool.schema["type"], "object", "{}", tool.name);
        }
        let names: Vec<_> = tools.iter().map(|tool| tool.name).collect();
        assert_eq!(
            names,
            [
                "search",
                "read_file",
                "find_symbol",
                "find_references",
                "list_issues",
                "get_issue",
                "recent_changes",
                "repo_map"
            ]
        );
    }

    #[test]
    fn search_returns_numbered_passages_and_their_sources() {
        let (_dir, tools) = setup();
        let answer = call(
            &tools,
            "search",
            json!({"query": "how do we verify a password"}),
        );
        assert!(!answer.is_error);
        assert!(
            answer.text.starts_with("[1] src/auth.rs:1-3"),
            "{}",
            answer.text
        );
        assert!(
            answer
                .sources
                .iter()
                .any(|source| source.reference == "src/auth.rs" && source.start_line == 1)
        );
    }

    #[test]
    fn a_search_with_no_match_says_so_instead_of_failing() {
        let (_dir, tools) = setup();
        let answer = call(&tools, "search", json!({"query": "zebra savanna"}));
        assert!(!answer.is_error);
        assert!(answer.text.contains("Nothing matched"));
        assert!(answer.sources.is_empty());
    }

    #[test]
    fn a_search_can_be_limited_to_issues() {
        let (_dir, tools) = setup();
        let answer = call(
            &tools,
            "search",
            json!({"query": "login timeout", "kinds": ["issue"]}),
        );
        assert!(answer.text.contains("issue 11111111-aaaa"));
        assert!(
            answer
                .sources
                .iter()
                .all(|source| source.kind == SourceKind::Issue)
        );
    }

    #[test]
    fn read_file_returns_numbered_lines_and_a_source() {
        let (_dir, tools) = setup();
        let answer = call(
            &tools,
            "read_file",
            json!({"path": "src/auth.rs", "start_line": 5, "end_line": 6}),
        );
        assert!(!answer.is_error);
        assert!(answer.text.contains("lines 5 to 6 of 7"), "{}", answer.text);
        assert!(answer.text.contains("    5 | pub fn sign_in"));
        assert_eq!(answer.sources[0].start_line, 5);
        assert_eq!(answer.sources[0].end_line, 6);
    }

    #[test]
    fn read_file_refuses_anything_outside_the_repository_and_secrets() {
        let (dir, tools) = setup();
        for path in [
            "../outside.txt",
            "/etc/passwd",
            "C:\\Windows\\win.ini",
            "src/../../x",
            ".env",
            "missing.rs",
            "src",
        ] {
            let answer = call(&tools, "read_file", json!({ "path": path }));
            assert!(answer.is_error, "{path} should be refused: {}", answer.text);
        }
        write(dir.path(), "src/key.pem", "x");
        assert!(call(&tools, "read_file", json!({"path": "src/key.pem"})).is_error);
    }

    #[test]
    fn read_file_stops_at_the_most_lines_one_read_gives() {
        let (dir, tools) = setup();
        let many: String = (1..=500).map(|n| format!("line {n}\n")).collect();
        write(dir.path(), "src/long.rs", &many);
        let answer = call(
            &tools,
            "read_file",
            json!({"path": "src/long.rs", "start_line": 1, "end_line": 500}),
        );
        assert!(answer.text.contains("lines 1 to 300 of 500"));
        assert!(answer.text.contains("Read on from the next line"));
    }

    #[test]
    fn find_symbol_and_find_references_answer_where_and_who() {
        let (_dir, tools) = setup();
        let definition = call(&tools, "find_symbol", json!({"name": "verify_password"}));
        assert!(definition.text.contains("src/auth.rs:1-3"));
        let uses = call(
            &tools,
            "find_references",
            json!({"name": "verify_password"}),
        );
        assert!(
            uses.text.contains("src/auth.rs:6  verify_password(p)"),
            "{}",
            uses.text
        );
        let none = call(
            &tools,
            "find_references",
            json!({"name": "nobody_calls_this"}),
        );
        assert!(none.text.contains("not used"));
    }

    #[test]
    fn issues_are_listed_filtered_and_read_with_their_commits() {
        let (_dir, tools) = setup();
        let todo = call(&tools, "list_issues", json!({"status": "todo"}));
        assert!(todo.text.contains("22222222-bbbb [Todo] Add refunds"));
        assert!(!todo.text.contains("login"));
        let one = call(&tools, "get_issue", json!({"id": "11111111"}));
        assert!(!one.is_error, "{}", one.text);
        assert!(one.text.contains("Fix login timeout in detail."));
        assert!(
            one.text
                .contains("abcdef12 Fix the login timeout (src/auth.rs)"),
            "{}",
            one.text
        );
        assert!(
            one.sources
                .iter()
                .any(|source| source.kind == SourceKind::Commit)
        );
        assert!(call(&tools, "get_issue", json!({"id": "nope"})).is_error);
    }

    #[test]
    fn recent_changes_lists_commits_with_their_day_and_files() {
        let (_dir, tools) = setup();
        let answer = call(&tools, "recent_changes", json!({"path": "auth"}));
        assert!(
            answer
                .text
                .contains("abcdef12 2025-10-26 Maya: Fix the login timeout (src/auth.rs)"),
            "{}",
            answer.text
        );
        assert!(
            call(&tools, "recent_changes", json!({"path": "nothing-here"}))
                .text
                .contains("No commits")
        );
    }

    #[test]
    fn the_repository_map_lists_folders_with_counts_and_languages() {
        let (_dir, tools) = setup();
        let map = call(&tools, "repo_map", json!({})).text;
        assert!(map.contains("src/ \u{2014} 2 files (Rust 2)"), "{map}");
        assert!(map.contains("docs/ \u{2014} 1 files (docs 1)"), "{map}");
        assert!(!map.contains(".env"));
    }

    #[test]
    fn an_unknown_tool_or_a_missing_argument_is_an_error_the_model_can_read() {
        let (_dir, tools) = setup();
        assert!(call(&tools, "delete_everything", json!({})).is_error);
        let missing = call(&tools, "search", json!({}));
        assert!(missing.is_error);
        assert!(missing.text.contains("query"));
    }

    #[test]
    fn a_very_long_answer_is_cut_and_says_so() {
        let text = cut("x".repeat(MAX_ANSWER_CHARS + 500));
        assert!(text.len() < MAX_ANSWER_CHARS + 200);
        assert!(text.ends_with("narrow the question.]"));
    }

    #[test]
    fn days_are_told_as_dates() {
        assert_eq!(day(0), "unknown date");
        assert_eq!(day(1_761_436_800), "2025-10-26");
        assert_eq!(day(86_400 * 59 + 3), "1970-03-01");
    }
}
