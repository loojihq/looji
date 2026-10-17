//! The cached index of a project's repository: its file tree, the SHA-256 of each source file, and the symbols read
//! from it. A refresh reads every file to hash it, but only parses the ones whose hash changed, or whose cached
//! parse came from an older parser. A file that's gone loses its row, and its symbols go with it.

use std::{
    collections::{HashMap, HashSet},
    fs,
    path::Path,
};

use looji_domain::{
    EntryKind, Language, ProjectId, Symbol, SymbolKind, TreeNode, build_tree, sort_symbols,
};
use looji_symbols::{Extractor, ExtractorError, MAX_FILE_BYTES, SkipReason};
use rusqlite::{Transaction, params};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::{
    SqliteStore,
    file_tree::{ScanError, scan_repo_tree},
};

/// Bumped whenever the parser or a symbol query changes. Cached parses from an older version are read again.
pub const PARSER_VERSION: i64 = 1;

#[derive(Debug, Error)]
pub enum IndexError {
    #[error(transparent)]
    Scan(#[from] ScanError),
    #[error(transparent)]
    Extractor(#[from] ExtractorError),
    #[error("the index could not be saved: {0}")]
    Storage(String),
}

/// A file that produced no symbols, and why, as text. Cached reasons are kept as the text they were saved in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileSkip {
    pub path: String,
    pub reason: String,
}

impl FileSkip {
    fn new(path: &str, reason: &SkipReason) -> Self {
        Self {
            path: path.to_string(),
            reason: reason.to_string(),
        }
    }
}

/// What a refresh did. `parsed` is the number of files sent to the parser. A refresh of an unchanged repository
/// parses nothing.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct IndexReport {
    /// Source files found in the repository.
    pub files_seen: usize,
    pub parsed: usize,
    /// Files whose cached symbols were still current.
    pub reused: usize,
    /// Files that were indexed before and aren't there now.
    pub removed: usize,
    /// Every file that produced no symbols, cached or new.
    pub skipped: Vec<FileSkip>,
}

/// The cached index as it was last refreshed.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CachedIndex {
    pub tree: Vec<TreeNode>,
    pub symbols: Vec<Symbol>,
    /// The source files that are indexed, in path order.
    pub files: Vec<String>,
}

impl SqliteStore {
    /// Brings the cache in line with the repository at `root`. Each file is read and hashed. Only files whose hash
    /// or parser version changed are parsed, and files that are gone are removed. The tree is replaced whole.
    /// Nothing is written unless the whole refresh succeeds.
    pub fn refresh_repo_index(
        &self,
        project: ProjectId,
        root: &Path,
    ) -> Result<IndexReport, IndexError> {
        let scanned = scan_repo_tree(root)?;
        let sources = source_files(&scanned.roots);
        let entries = flatten(&scanned.roots);
        let key = project.as_uuid().to_string();
        let stored = self.stored_index_files(&key)?;

        let transaction = crate::write_transaction(&self.connection).map_err(storage)?;
        let mut report = IndexReport::default();
        let mut extractor: Option<Extractor> = None;
        let mut current = HashSet::new();

        for (path, language) in &sources {
            report.files_seen += 1;
            current.insert(path.as_str());
            let bytes = match fs::read(root.join(path)) {
                Ok(bytes) => bytes,
                Err(err) => {
                    let reason = SkipReason::Unreadable(err.to_string());
                    write_file(&transaction, &key, path, "", Some(&reason), &[])?;
                    report.skipped.push(FileSkip::new(path, &reason));
                    continue;
                }
            };
            let hash = hex(&Sha256::digest(&bytes));

            if let Some(cached) = stored.get(path)
                && cached.hash == hash
                && cached.parser_version == PARSER_VERSION
            {
                report.reused += 1;
                if let Some(reason) = &cached.skip_reason {
                    report.skipped.push(FileSkip {
                        path: path.clone(),
                        reason: reason.clone(),
                    });
                }
                continue;
            }

            let outcome = parse_bytes(bytes, *language, path, &mut extractor, &mut report)?;
            match outcome {
                Ok(symbols) => write_file(&transaction, &key, path, &hash, None, &symbols)?,
                Err(reason) => {
                    write_file(&transaction, &key, path, &hash, Some(&reason), &[])?;
                    report.skipped.push(FileSkip::new(path, &reason));
                }
            }
        }

        for path in stored
            .keys()
            .filter(|path| !current.contains(path.as_str()))
        {
            // Deleting the row removes its symbols too, through the composite foreign key.
            transaction
                .execute(
                    "DELETE FROM index_files WHERE project_id = ?1 AND path = ?2",
                    params![key, path],
                )
                .map_err(storage)?;
            report.removed += 1;
        }

        transaction
            .execute(
                "DELETE FROM index_entries WHERE project_id = ?1",
                params![key],
            )
            .map_err(storage)?;
        for (path, kind) in &entries {
            transaction
                .execute(
                    "INSERT INTO index_entries (project_id, path, kind) VALUES (?1, ?2, ?3)",
                    params![key, path, entry_kind_label(*kind)],
                )
                .map_err(storage)?;
        }

        transaction.commit().map_err(storage)?;
        Ok(report)
    }

    /// The index as it was last refreshed. A project that has never been indexed has an empty one.
    pub fn load_repo_index(&self, project: ProjectId) -> Result<CachedIndex, IndexError> {
        let key = project.as_uuid().to_string();

        let mut statement = self
            .connection
            .prepare("SELECT path, kind FROM index_entries WHERE project_id = ?1 ORDER BY path")
            .map_err(storage)?;
        let entries: Vec<(String, EntryKind)> = statement
            .query_map(params![key], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(storage)?
            .map(|row| {
                let (path, kind) = row.map_err(storage)?;
                Ok((path, parse_entry_kind(&kind)))
            })
            .collect::<Result<_, IndexError>>()?;
        let tree = build_tree(entries.iter().map(|(path, kind)| (path.as_str(), *kind)));

        let mut statement = self
            .connection
            .prepare(
                "SELECT path, name, kind, start_line, end_line FROM index_symbols
                 WHERE project_id = ?1",
            )
            .map_err(storage)?;
        let mut symbols: Vec<Symbol> = statement
            .query_map(params![key], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, u32>(3)?,
                    row.get::<_, u32>(4)?,
                ))
            })
            .map_err(storage)?
            .map(|row| {
                let (path, name, kind, start_line, end_line) = row.map_err(storage)?;
                Ok(Symbol {
                    name,
                    kind: parse_symbol_kind(&kind),
                    path,
                    start_line,
                    end_line,
                })
            })
            .collect::<Result<_, IndexError>>()?;
        sort_symbols(&mut symbols);

        let mut statement = self
            .connection
            .prepare("SELECT path FROM index_files WHERE project_id = ?1 ORDER BY path")
            .map_err(storage)?;
        let files = statement
            .query_map(params![key], |row| row.get::<_, String>(0))
            .map_err(storage)?
            .collect::<Result<Vec<String>, _>>()
            .map_err(storage)?;

        Ok(CachedIndex {
            tree,
            symbols,
            files,
        })
    }

    fn stored_index_files(&self, key: &str) -> Result<HashMap<String, StoredFile>, IndexError> {
        let mut statement = self
            .connection
            .prepare("SELECT path, hash, parser_version, skip_reason FROM index_files WHERE project_id = ?1")
            .map_err(storage)?;
        let rows = statement
            .query_map(params![key], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    StoredFile {
                        hash: row.get(1)?,
                        parser_version: row.get(2)?,
                        skip_reason: row.get(3)?,
                    },
                ))
            })
            .map_err(storage)?;
        rows.collect::<Result<HashMap<_, _>, _>>().map_err(storage)
    }
}

/// One file's cached row.
struct StoredFile {
    hash: String,
    parser_version: i64,
    skip_reason: Option<String>,
}

/// Parses one file's bytes. The inner result is the file's skip reason, if it has one. The outer one is an error
/// that stops the refresh, which only happens when the parser itself can't be built.
fn parse_bytes(
    bytes: Vec<u8>,
    language: Language,
    path: &str,
    extractor: &mut Option<Extractor>,
    report: &mut IndexReport,
) -> Result<Result<Vec<Symbol>, SkipReason>, IndexError> {
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Ok(Err(SkipReason::TooLarge {
            bytes: bytes.len() as u64,
        }));
    }
    let Ok(source) = String::from_utf8(bytes) else {
        return Ok(Err(SkipReason::NotUtf8));
    };
    if extractor.is_none() {
        *extractor = Some(Extractor::new()?);
    }
    let extractor = extractor.as_mut().expect("the extractor was just created");
    report.parsed += 1;
    Ok(extractor.extract(language, path, &source))
}

/// Replaces a file's row, and with it the file's symbols.
fn write_file(
    transaction: &Transaction,
    key: &str,
    path: &str,
    hash: &str,
    skip: Option<&SkipReason>,
    symbols: &[Symbol],
) -> Result<(), IndexError> {
    transaction
        .execute(
            "DELETE FROM index_files WHERE project_id = ?1 AND path = ?2",
            params![key, path],
        )
        .map_err(storage)?;
    transaction
        .execute(
            "INSERT INTO index_files (project_id, path, hash, parser_version, skip_reason)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                key,
                path,
                hash,
                PARSER_VERSION,
                skip.map(|reason| reason.to_string()),
            ],
        )
        .map_err(storage)?;
    for symbol in symbols {
        transaction
            .execute(
                "INSERT INTO index_symbols (project_id, path, name, kind, start_line, end_line)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    key,
                    path,
                    symbol.name,
                    symbol_kind_label(symbol.kind),
                    symbol.start_line,
                    symbol.end_line,
                ],
            )
            .map_err(storage)?;
    }
    Ok(())
}

/// Every source file in the tree, with its language.
fn source_files(nodes: &[TreeNode]) -> Vec<(String, Language)> {
    let mut files = Vec::new();
    collect_sources(nodes, &mut files);
    files
}

fn collect_sources(nodes: &[TreeNode], files: &mut Vec<(String, Language)>) {
    for node in nodes {
        match node.kind {
            EntryKind::Directory => collect_sources(&node.children, files),
            EntryKind::File => {
                if let Some(language) = Language::from_path(&node.path) {
                    files.push((node.path.clone(), language));
                }
            }
            EntryKind::Symlink => {}
        }
    }
}

fn flatten(nodes: &[TreeNode]) -> Vec<(String, EntryKind)> {
    let mut entries = Vec::new();
    for node in nodes {
        entries.push((node.path.clone(), node.kind));
        entries.extend(flatten(&node.children));
    }
    entries
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn storage(err: rusqlite::Error) -> IndexError {
    IndexError::Storage(err.to_string())
}

fn entry_kind_label(kind: EntryKind) -> &'static str {
    match kind {
        EntryKind::Directory => "directory",
        EntryKind::File => "file",
        EntryKind::Symlink => "symlink",
    }
}

fn parse_entry_kind(text: &str) -> EntryKind {
    match text {
        "directory" => EntryKind::Directory,
        "symlink" => EntryKind::Symlink,
        _ => EntryKind::File,
    }
}

fn symbol_kind_label(kind: SymbolKind) -> &'static str {
    kind.label()
}

fn parse_symbol_kind(text: &str) -> SymbolKind {
    match text {
        "type" => SymbolKind::Type,
        "module" => SymbolKind::Module,
        _ => SymbolKind::Function,
    }
}
