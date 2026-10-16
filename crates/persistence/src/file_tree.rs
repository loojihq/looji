//! Walks a repository folder into its file tree. What's left out follows the repository's ignore files
//! (`.gitignore`, `.git/info/exclude`, and the global ignore file), plus the built-in list in the domain.
//!
//! Symbolic links are listed but never followed, so a link back up the tree can't make the walk recurse forever.
//! Nothing here touches the database. Callers run it off the UI thread, since a large or networked folder can take
//! a long time.

use std::{fs, io, path::Path};

use ignore::WalkBuilder;
use looji_domain::{EntryKind, TreeNode, build_tree, is_built_in_ignored};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ScanError {
    #[error("the folder can't be read ({0})")]
    Unreadable(String),
}

/// The tree, and the entries the walk had to skip because they couldn't be read. Skipped entries don't stop the scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScannedTree {
    pub roots: Vec<TreeNode>,
    pub skipped: Vec<String>,
}

/// Lists the repository at `root`. The root itself must be readable, otherwise there's no tree.
pub fn scan_repo_tree(root: &Path) -> Result<ScannedTree, ScanError> {
    fs::read_dir(root).map_err(|err: io::Error| ScanError::Unreadable(err.to_string()))?;

    let walk = WalkBuilder::new(root)
        .hidden(false)
        .require_git(false)
        .git_ignore(true)
        .git_exclude(true)
        .git_global(true)
        // Only this folder's ignore files apply. A repository nested in another must not inherit the outer rules.
        .parents(false)
        .follow_links(false)
        .sort_by_file_name(|a, b| a.cmp(b))
        .filter_entry(|entry| {
            let is_dir = entry.file_type().is_some_and(|kind| kind.is_dir());
            entry.depth() == 0
                || !entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| is_built_in_ignored(name, is_dir))
        })
        .build();

    let mut entries: Vec<(String, EntryKind)> = Vec::new();
    let mut skipped = Vec::new();
    for result in walk {
        match result {
            Ok(entry) => {
                if entry.depth() == 0 {
                    continue;
                }
                let Ok(relative) = entry.path().strip_prefix(root) else {
                    continue;
                };
                let path = relative
                    .components()
                    .map(|part| part.as_os_str().to_string_lossy())
                    .collect::<Vec<_>>()
                    .join("/");
                let kind = if entry.path_is_symlink() {
                    EntryKind::Symlink
                } else if entry.file_type().is_some_and(|kind| kind.is_dir()) {
                    EntryKind::Directory
                } else {
                    EntryKind::File
                };
                entries.push((path, kind));
            }
            Err(err) => skipped.push(err.to_string()),
        }
    }

    Ok(ScannedTree {
        roots: build_tree(entries.iter().map(|(path, kind)| (path.as_str(), *kind))),
        skipped,
    })
}
