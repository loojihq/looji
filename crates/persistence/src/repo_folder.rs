//! Checks a chosen folder on disk: it exists, it's a folder, and it's the top folder of a git repository. A folder
//! inside a WSL distribution (`\\wsl.localhost\…`) is checked through [`crate::wsl`].
//!
//! Nothing here touches the database. Callers run it off the UI thread, since a network drive can block for a
//! long time. Paths are used as given. They are not canonicalised, so Windows doesn't show `\?\` prefixes.

use std::{fs, io, path::Path};

use looji_domain::{PathCheck, parse_wsl_path};

use crate::wsl;

/// Checks that `path` is the top folder of a git repository. Returns the first problem found.
///
/// A `.git` entry can be a folder, or a file in a linked worktree or submodule. Either counts.
pub fn check_repo_folder(path: &Path) -> Result<(), PathCheck> {
    match path.to_str().and_then(parse_wsl_path) {
        Some(wsl_path) => wsl::check_folder(path, &wsl_path),
        None => check_local_folder(path),
    }
}

/// Reads a file from a repository folder. A WSL file is read directly while its distribution runs. If that read
/// fails, it is read through the distribution. A stopped distribution is an error, not something that gets started.
pub fn read_repo_file(path: &Path) -> io::Result<Vec<u8>> {
    let Some(wsl_path) = path.to_str().and_then(parse_wsl_path) else {
        return fs::read(path);
    };
    wsl::resolve_running(&wsl_path.distro)
        .map_err(|reason| io::Error::other(reason.to_string()))?;
    match fs::read(path) {
        Ok(bytes) => Ok(bytes),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Err(err),
        Err(_) => wsl::read_through_distro(&wsl_path.distro, &wsl_path.linux_path),
    }
}

/// The check for a folder on this machine, or one that Windows reaches directly.
pub(crate) fn check_local_folder(path: &Path) -> Result<(), PathCheck> {
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
