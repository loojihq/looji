//! Checks a chosen folder on disk: it exists, it's a folder, and it's the top folder of a git repository.
//!
//! Nothing here touches the database. Callers run it off the UI thread, since a network drive can block for a
//! long time. Paths are used as given. They are not canonicalised, so Windows doesn't show `\?\` prefixes.

use std::{fs, io, path::Path};

use looji_domain::PathCheck;

/// Checks that `path` is the top folder of a git repository. Returns the first problem found.
///
/// A `.git` entry can be a folder, or a file in a linked worktree or submodule. Either counts.
pub fn check_repo_folder(path: &Path) -> Result<(), PathCheck> {
    let metadata = fs::metadata(path).map_err(from_io)?;
    if !metadata.is_dir() {
        return Err(PathCheck::NotADirectory);
    }
    // Listing the folder reports permission problems that reading its metadata doesn't.
    fs::read_dir(path).map_err(from_io)?;
    match fs::metadata(path.join(".git")) {
        Ok(git) if git.is_dir() || git.is_file() => Ok(()),
        Ok(_) => Err(PathCheck::NotAGitRepository),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Err(PathCheck::NotAGitRepository),
        Err(err) => Err(from_io(err)),
    }
}

fn from_io(err: io::Error) -> PathCheck {
    match err.kind() {
        io::ErrorKind::NotFound => PathCheck::Missing,
        _ => PathCheck::Unreadable(err.to_string()),
    }
}
