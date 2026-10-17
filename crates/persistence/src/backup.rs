//! Backups taken before an upgrade changes the schema.
//!
//! When pending migrations would change a database that already holds data, a copy is made first, in a `backups`
//! folder beside the database. If a migration fails, the copy is put back, so a failed upgrade leaves the data exactly
//! as it was. After a successful upgrade, a marker file records the backup. It stays until the new version has
//! started, and [`confirm_startup`] then clears the marker and removes older backups, keeping the newest.

use std::{
    fs, io,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use rusqlite::Connection;

use crate::{Error, Result};

const BACKUP_PREFIX: &str = "looji-before-v";

/// A copy of the database as it was before an upgrade.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpgradeBackup {
    pub path: PathBuf,
}

/// The folder backups are kept in: `backups` next to the database file.
pub fn backups_dir(database_path: &Path) -> PathBuf {
    database_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("backups")
}

/// The marker that says an upgrade has been done but the new version hasn't yet confirmed it started.
fn marker_path(database_path: &Path) -> PathBuf {
    let mut name = database_path
        .file_name()
        .map(|name| name.to_os_string())
        .unwrap_or_default();
    name.push(".upgrade-pending");
    database_path.with_file_name(name)
}

/// Copies the database to a timestamped file in the backups folder. `pending` names the versions about to be
/// applied, which go into the file name. The copy is a consistent snapshot, whatever the write-ahead log holds.
pub(crate) fn create(
    connection: &Connection,
    database_path: &Path,
    pending: &[u32],
) -> Result<UpgradeBackup> {
    let folder = backups_dir(database_path);
    fs::create_dir_all(&folder).map_err(|err| backup_error(&folder, err))?;
    let first = pending.first().copied().unwrap_or(0);
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis())
        .unwrap_or(0);
    let mut path = folder.join(format!("{BACKUP_PREFIX}{first}-{millis}.db"));
    let mut attempt = 1;
    while path.exists() {
        path = folder.join(format!("{BACKUP_PREFIX}{first}-{millis}-{attempt}.db"));
        attempt += 1;
    }
    let target = path
        .to_str()
        .ok_or_else(|| backup_error(&path, io::Error::other("the path isn't valid text")))?;
    connection.execute("VACUUM INTO ?1", [target])?;
    Ok(UpgradeBackup { path })
}

/// Puts a backup back in place of the database. The write-ahead log and its index belong to the database that is
/// being replaced, so they go first. The connection must already be closed.
pub(crate) fn restore(database_path: &Path, backup: &UpgradeBackup) -> io::Result<()> {
    for suffix in ["-wal", "-shm"] {
        let mut name = database_path
            .file_name()
            .map(|name| name.to_os_string())
            .unwrap_or_default();
        name.push(suffix);
        match fs::remove_file(database_path.with_file_name(name)) {
            Ok(()) => {}
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => return Err(err),
        }
    }
    fs::copy(&backup.path, database_path).map(|_| ())
}

/// Records that an upgrade finished and its backup is waiting to be confirmed.
pub(crate) fn mark_pending(database_path: &Path, backup: &UpgradeBackup) -> Result<()> {
    let marker = marker_path(database_path);
    fs::write(&marker, backup.path.to_string_lossy().as_bytes())
        .map_err(|err| backup_error(&marker, err))
}

/// The backup waiting to be confirmed, if an upgrade ran and the new version hasn't confirmed its start yet.
pub fn unconfirmed_backup(database_path: &Path) -> Option<UpgradeBackup> {
    let text = fs::read_to_string(marker_path(database_path)).ok()?;
    let path = PathBuf::from(text.trim());
    path.exists().then_some(UpgradeBackup { path })
}

/// Called once the new version has started. Clears the pending marker and removes backups older than the newest.
/// The newest stays, so a bad upgrade that only shows up later can still be undone by hand. Without a marker,
/// nothing happens.
pub fn confirm_startup(database_path: &Path) -> Result<()> {
    let marker = marker_path(database_path);
    if !marker.exists() {
        return Ok(());
    }
    fs::remove_file(&marker).map_err(|err| backup_error(&marker, err))?;

    let folder = backups_dir(database_path);
    let Ok(entries) = fs::read_dir(&folder) else {
        return Ok(());
    };
    let mut backups: Vec<(SystemTime, PathBuf)> = entries
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with(BACKUP_PREFIX)
        })
        .filter_map(|entry| Some((entry.metadata().ok()?.modified().ok()?, entry.path())))
        .collect();
    backups.sort();
    backups.pop();
    for (_, old) in backups {
        let _ = fs::remove_file(old);
    }
    Ok(())
}

fn backup_error(path: &Path, err: io::Error) -> Error {
    Error::Backup {
        path: path.to_path_buf(),
        reason: err.to_string(),
    }
}
