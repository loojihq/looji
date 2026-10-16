//! Indexes the symbols of every source file in a repository's file tree. A file that can't be read or parsed is
//! logged and recorded, and the rest of the index still builds.

use std::{fs, path::Path};

use looji_domain::{EntryKind, Language, Symbol, TreeNode, sort_symbols};

use crate::extract::{Extractor, ExtractorError, SkipReason};

/// Largest file parsed, in bytes. Anything bigger is skipped, since a generated file that big rarely has useful
/// symbols and slows the index down.
pub const MAX_FILE_BYTES: u64 = 1024 * 1024;

/// A file that produced no symbols, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedFile {
    pub path: String,
    pub reason: SkipReason,
}

/// The symbols of a repository, in the domain's stable order, and the files that were skipped.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SymbolIndex {
    pub symbols: Vec<Symbol>,
    /// How many files were parsed, whether or not they had symbols.
    pub files_indexed: usize,
    pub skipped: Vec<SkippedFile>,
}

/// Indexes the source files in `nodes`, which are paths under `root`. Symbolic links and folders aren't read, and
/// neither are files in languages looji doesn't know.
pub fn index_tree(root: &Path, nodes: &[TreeNode]) -> Result<SymbolIndex, ExtractorError> {
    let mut files = Vec::new();
    collect_files(nodes, &mut files);

    let mut extractor = Extractor::new()?;
    let mut index = SymbolIndex::default();
    for (path, language) in files {
        match index_file(root, &path, language, &mut extractor) {
            Ok(mut symbols) => {
                index.files_indexed += 1;
                index.symbols.append(&mut symbols);
            }
            Err(reason) => {
                eprintln!("looji: skipped {path} while indexing symbols: {reason}");
                index.skipped.push(SkippedFile { path, reason });
            }
        }
    }
    sort_symbols(&mut index.symbols);
    Ok(index)
}

fn collect_files(nodes: &[TreeNode], files: &mut Vec<(String, Language)>) {
    for node in nodes {
        match node.kind {
            EntryKind::Directory => collect_files(&node.children, files),
            EntryKind::File => {
                if let Some(language) = Language::from_path(&node.path) {
                    files.push((node.path.clone(), language));
                }
            }
            EntryKind::Symlink => {}
        }
    }
}

fn index_file(
    root: &Path,
    path: &str,
    language: Language,
    extractor: &mut Extractor,
) -> Result<Vec<Symbol>, SkipReason> {
    let full = root.join(path);
    let size = fs::metadata(&full)
        .map_err(|err| SkipReason::Unreadable(err.to_string()))?
        .len();
    if size > MAX_FILE_BYTES {
        return Err(SkipReason::TooLarge { bytes: size });
    }
    let bytes = fs::read(&full).map_err(|err| SkipReason::Unreadable(err.to_string()))?;
    let source = String::from_utf8(bytes).map_err(|_| SkipReason::NotUtf8)?;
    extractor.extract(language, path, &source)
}
