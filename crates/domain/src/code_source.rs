//! A project's connection to a local repository folder: the rules for the path the user types, the reasons a
//! connection can be broken, and the status a project shows.
//!
//! Nothing here touches the disk. The filesystem check lives in persistence, and it reports its result as a
//! `PathCheck`, so these rules can be tested without a folder.
//!
//! Decisions this module enforces:
//! - A path is trimmed, and a blank one is refused, so no empty path is ever stored.
//! - A saved path is kept when its folder goes away. A broken connection says so, and only Disconnect removes the
//!   path. A drive that comes back, or a folder that's moved back, reconnects without being chosen again.

use std::fmt;

use thiserror::Error;

/// Why a path can't be used as a repository. Each reason has its own message, so the user knows what to fix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathCheck {
    /// Nothing exists at the path.
    Missing,
    /// The path is a file, not a folder.
    NotADirectory,
    /// The folder has no `.git` entry. That also covers a folder inside a repository, since only the top folder of
    /// a repository has one.
    NotAGitRepository,
    /// The folder exists but can't be read. The reason is the system's message, which doesn't contain file contents.
    Unreadable(String),
}

impl fmt::Display for PathCheck {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PathCheck::Missing => write!(
                f,
                "That folder doesn't exist. Check the path, or choose the folder again."
            ),
            PathCheck::NotADirectory => write!(
                f,
                "That path is a file. Choose the folder that holds the repository."
            ),
            PathCheck::NotAGitRepository => write!(
                f,
                "That folder has no .git entry, so it isn't the top folder of a git repository."
            ),
            PathCheck::Unreadable(reason) => write!(
                f,
                "looji can't read that folder ({reason}). Check that you have access to it."
            ),
        }
    }
}

/// Why the path the user typed was refused before it was checked.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CodeSourceError {
    #[error("Enter the folder's path.")]
    EmptyPath,
}

/// Trims the path the user typed. A blank path is refused.
pub fn clean_path_input(text: &str) -> Result<String, CodeSourceError> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(CodeSourceError::EmptyPath);
    }
    Ok(trimmed.to_string())
}

/// What a project shows about its repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodeSourceStatus {
    /// No folder has been chosen.
    NotConnected,
    /// The chosen folder is a git repository.
    Connected { path: String },
    /// A folder was chosen, but it can't be used now. The path is kept, so it can be located again.
    Broken { path: String, reason: PathCheck },
}

/// The status for a stored path and the result of checking it. Without a stored path there's nothing to check.
pub fn status_for(path: Option<String>, check: Result<(), PathCheck>) -> CodeSourceStatus {
    match (path, check) {
        (None, _) => CodeSourceStatus::NotConnected,
        (Some(path), Ok(())) => CodeSourceStatus::Connected { path },
        (Some(path), Err(reason)) => CodeSourceStatus::Broken { path, reason },
    }
}

#[cfg(test)]
mod tests {
    // Not `use super::*`: gpui's `test` macro would shadow `#[test]`.
    use crate::code_source::{
        CodeSourceError, CodeSourceStatus, PathCheck, clean_path_input, status_for,
    };

    #[test]
    fn an_empty_or_blank_path_is_refused() {
        assert_eq!(clean_path_input(""), Err(CodeSourceError::EmptyPath));
        assert_eq!(clean_path_input("   \t "), Err(CodeSourceError::EmptyPath));
    }

    #[test]
    fn a_path_is_trimmed_before_it_is_stored() {
        assert_eq!(
            clean_path_input("  C:\\dev\\looji  "),
            Ok("C:\\dev\\looji".to_string())
        );
    }

    #[test]
    fn no_stored_path_is_not_connected_whatever_the_check_says() {
        assert_eq!(status_for(None, Ok(())), CodeSourceStatus::NotConnected);
        assert_eq!(
            status_for(None, Err(PathCheck::Missing)),
            CodeSourceStatus::NotConnected
        );
    }

    #[test]
    fn a_stored_path_that_checks_out_is_connected() {
        assert_eq!(
            status_for(Some("C:\\repo".to_string()), Ok(())),
            CodeSourceStatus::Connected {
                path: "C:\\repo".to_string()
            }
        );
    }

    #[test]
    fn a_stored_path_that_fails_the_check_is_broken_and_keeps_the_path() {
        assert_eq!(
            status_for(Some("D:\\gone".to_string()), Err(PathCheck::Missing)),
            CodeSourceStatus::Broken {
                path: "D:\\gone".to_string(),
                reason: PathCheck::Missing
            }
        );
    }

    #[test]
    fn each_failure_has_its_own_message() {
        let messages: Vec<String> = [
            PathCheck::Missing,
            PathCheck::NotADirectory,
            PathCheck::NotAGitRepository,
            PathCheck::Unreadable("access denied".to_string()),
        ]
        .iter()
        .map(ToString::to_string)
        .collect();
        for (index, message) in messages.iter().enumerate() {
            assert!(
                messages[..index].iter().all(|earlier| earlier != message),
                "{message}"
            );
        }
        assert!(messages[2].contains(".git"));
        assert!(messages[3].contains("access denied"));
    }
}
