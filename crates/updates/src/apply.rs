//! Puts a verified update in place of the running program, and starts it.
//!
//! On Windows a running program can't be overwritten, but it can be renamed. So the current program is renamed aside,
//! the new one is copied to its name, and the new one is started. If any step fails, the old program is renamed back,
//! so the previous version still starts. The old file stays until the new version has started, and the new version
//! removes it then.

use std::{
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ApplyError {
    #[error("the update file can't be read ({0}). Nothing was changed.")]
    StagedUnreadable(String),
    #[error("the current program can't be moved aside ({0}). Nothing was changed.")]
    MoveAside(String),
    #[error("the new program couldn't be put in place ({0}). The previous version was restored.")]
    CopyFailed(String),
    #[error("the new program couldn't be put in place ({copy}), and the previous version couldn't be restored ({restore}). It is at {}.", previous.display())]
    RestoreFailed {
        copy: String,
        restore: String,
        previous: PathBuf,
    },
    #[error("the new version couldn't be started ({0}). The previous version was restored.")]
    LaunchFailed(String),
}

/// A replacement that has been made: where the new program is, and where the old one was moved to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Replacement {
    pub target: PathBuf,
    pub previous: PathBuf,
}

/// Where the old program is kept: the program's own name with `.old` added.
pub fn previous_path(target: &Path) -> PathBuf {
    let mut name: OsString = target.file_name().map(OsString::from).unwrap_or_default();
    name.push(".old");
    target.with_file_name(name)
}

/// Replaces `target` with the file at `staged`. The staged file is checked first, so a bad one changes nothing. If
/// the copy fails, the old program is put back.
pub fn replace_executable(target: &Path, staged: &Path) -> Result<Replacement, ApplyError> {
    let staged_size = fs::metadata(staged)
        .map_err(|err| ApplyError::StagedUnreadable(err.to_string()))?
        .len();
    let previous = previous_path(target);

    // A leftover from an earlier update would be in the way. If it can't be removed, the rename below fails too.
    let _ = fs::remove_file(&previous);
    fs::rename(target, &previous).map_err(|err| ApplyError::MoveAside(err.to_string()))?;

    let copied = fs::copy(staged, target).and_then(|_| {
        let size = fs::metadata(target)?.len();
        if size == staged_size {
            Ok(())
        } else {
            Err(io::Error::other("the copy is not the size of the update"))
        }
    });
    if let Err(copy) = copied {
        let _ = fs::remove_file(target);
        return match fs::rename(&previous, target) {
            Ok(()) => Err(ApplyError::CopyFailed(copy.to_string())),
            Err(restore) => Err(ApplyError::RestoreFailed {
                copy: copy.to_string(),
                restore: restore.to_string(),
                previous,
            }),
        };
    }
    Ok(Replacement {
        target: target.to_path_buf(),
        previous,
    })
}

/// Undoes a replacement: the new program is removed and the old one put back.
pub fn roll_back(replacement: &Replacement) -> io::Result<()> {
    let _ = fs::remove_file(&replacement.target);
    fs::rename(&replacement.previous, &replacement.target)
}

/// Replaces `target`, then starts the new program with `args`. If it can't be started, the replacement is undone.
/// On success the caller should close the running program, since the new one is on its way up.
pub fn replace_and_start(
    target: &Path,
    staged: &Path,
    args: &[String],
) -> Result<Replacement, ApplyError> {
    let replacement = replace_executable(target, staged)?;
    if let Err(err) = Command::new(target).args(args).spawn() {
        return match roll_back(&replacement) {
            Ok(()) => Err(ApplyError::LaunchFailed(err.to_string())),
            Err(restore) => Err(ApplyError::RestoreFailed {
                copy: err.to_string(),
                restore: restore.to_string(),
                previous: replacement.previous,
            }),
        };
    }
    Ok(replacement)
}

/// Removes the old program after an update. The new version calls this once it has started, so until then the old
/// one is still there to go back to. A missing file is fine.
pub fn remove_previous(target: &Path) -> io::Result<()> {
    match fs::remove_file(previous_path(target)) {
        Err(err) if err.kind() != io::ErrorKind::NotFound => Err(err),
        _ => Ok(()),
    }
}

/// Waits for the process `pid` to exit, up to `timeout`. The new version calls this so it doesn't open the database
/// while the old version is still closing it.
#[cfg(windows)]
pub fn wait_for_exit(pid: u32, timeout: Duration) {
    use windows::Win32::{
        Foundation::CloseHandle,
        System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject},
    };
    // SAFETY: the handle is opened, waited on, and closed here, and isn't used anywhere else.
    unsafe {
        if let Ok(handle) = OpenProcess(PROCESS_SYNCHRONIZE, false, pid) {
            WaitForSingleObject(handle, timeout.as_millis().min(u128::from(u32::MAX)) as u32);
            let _ = CloseHandle(handle);
        }
    }
}

#[cfg(not(windows))]
pub fn wait_for_exit(_pid: u32, timeout: Duration) {
    std::thread::sleep(timeout.min(Duration::from_secs(2)));
}
