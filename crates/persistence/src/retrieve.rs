//! Finds the code a task is about: the cached symbols that match its words, read from the repository and packed into
//! a token budget. The ranking rules are in the domain. This file only gathers the candidates.

use std::{collections::HashMap, fs, path::Path};

use looji_domain::{Hunk, ProjectId, pack_hunks, score_symbol, task_terms};

use crate::{SqliteStore, repo_index::IndexError};

/// Longest hunk taken from one symbol. A class with many methods is shown in pieces, not whole.
const MAX_HUNK_LINES: u32 = 120;
/// Lines taken from the top of a file that matches by name but has no indexed symbols.
const FILE_HEAD_LINES: u32 = 40;

impl SqliteStore {
    /// The hunks for a task, best first. Empty when nothing in the cached index matches the task's words, so an
    /// unrelated task never gets unrelated code. Reads the source lines from `root`, so the index should be
    /// refreshed first.
    pub fn retrieve_hunks(
        &self,
        project: ProjectId,
        root: &Path,
        title: &str,
        description: &str,
        budget_tokens: usize,
    ) -> Result<Vec<Hunk>, IndexError> {
        let terms = task_terms(&format!("{title} {description}"));
        if terms.is_empty() {
            return Ok(Vec::new());
        }
        let index = self.load_repo_index(project)?;
        let mut lines = FileLines::new(root);
        let mut candidates = Vec::new();

        for symbol in &index.symbols {
            let score = score_symbol(&terms, &symbol.name, &symbol.path);
            if score == 0 {
                continue;
            }
            let end = symbol.end_line.min(symbol.start_line + MAX_HUNK_LINES - 1);
            if let Some(hunk) = lines.hunk(&symbol.path, symbol.start_line, end, score) {
                candidates.push(hunk);
            }
        }

        let with_symbols: std::collections::HashSet<&str> = index
            .symbols
            .iter()
            .map(|symbol| symbol.path.as_str())
            .collect();
        for path in &index.files {
            if with_symbols.contains(path.as_str()) {
                continue;
            }
            let name = Path::new(path)
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or_default();
            let score = score_symbol(&terms, name, path);
            if score == 0 {
                continue;
            }
            if let Some(hunk) = lines.hunk(path, 1, FILE_HEAD_LINES, score) {
                candidates.push(hunk);
            }
        }

        Ok(pack_hunks(
            candidates,
            budget_tokens,
            looji_domain::MAX_HUNKS,
        ))
    }
}

/// Reads each file at most once. A file that can't be read as UTF-8 text gives no hunks.
struct FileLines<'a> {
    root: &'a Path,
    files: HashMap<String, Option<Vec<String>>>,
}

impl<'a> FileLines<'a> {
    fn new(root: &'a Path) -> Self {
        Self {
            root,
            files: HashMap::new(),
        }
    }

    fn hunk(&mut self, path: &str, start: u32, end: u32, score: u32) -> Option<Hunk> {
        let root = self.root;
        let lines = self
            .files
            .entry(path.to_string())
            .or_insert_with(|| {
                fs::read_to_string(root.join(path))
                    .ok()
                    .map(|text| text.lines().map(str::to_owned).collect())
            })
            .as_ref()?;
        let total = lines.len() as u32;
        if start == 0 || start > total {
            return None;
        }
        let end = end.min(total).max(start);
        let text = lines[(start - 1) as usize..end as usize].join("\n");
        Some(Hunk {
            path: path.to_string(),
            start_line: start,
            end_line: end,
            score,
            text,
        })
    }
}
