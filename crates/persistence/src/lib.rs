//! SQLite storage: where the database lives, how it is opened, and how its schema evolves.

pub mod backup;
pub mod file_tree;
pub mod icons;
mod migrations;
mod project_schedule;
pub mod repo_folder;
pub mod repo_index;
mod repositories;
mod retrieve;
mod schema;
mod status_visibility;
mod store;
mod workspace_delete;
pub mod wsl;

pub use workspace_delete::{
    DeleteError, DeletedWorkspace, WorkspaceSummary, workspace_scoped_tables,
};

use std::{
    fs,
    path::{Path, PathBuf},
};

pub use backup::{UpgradeBackup, backups_dir, confirm_startup, unconfirmed_backup};
pub use migrations::{Migration, migrate};
pub use project_schedule::ScheduleSaveError;
pub use repositories::task_move::MoveError;
pub use rusqlite::Connection;
pub use status_visibility::VisibilityError;
pub use store::SqliteStore;
use thiserror::Error;

use migrations::migrate_with_hook;

const DATABASE_FILE_NAME: &str = "looji.db";

#[derive(Debug, Error)]
pub enum Error {
    #[error("could not create the data directory {}: {source}", path.display())]
    CreateDirectory {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("could not open the database: {source}")]
    Open {
        #[source]
        source: rusqlite::Error,
    },
    #[error("could not set up the database: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error(
        "the database is at schema v{version}, but this version of looji only knows up to v{latest}"
    )]
    DatabaseAhead { version: u32, latest: u32 },
    #[error("v{version} is recorded as applied but this version of looji does not have it")]
    MissingMigration { version: u32 },
    #[error("invalid migration list: {0}")]
    InvalidMigrations(String),
    #[error("migration v{version} ({name}) failed and was rolled back: {reason}")]
    MigrationFailed {
        version: u32,
        name: String,
        reason: String,
    },
    #[error("the database could not be backed up at {}: {reason}", path.display())]
    Backup { path: PathBuf, reason: String },
    #[error("the upgrade failed ({cause}), so your data was restored from the backup at {}", backup.display())]
    UpgradeRolledBack { cause: String, backup: PathBuf },
    #[error("the upgrade failed ({cause}), and the backup at {} could not be put back: {reason}", backup.display())]
    RestoreFailed {
        cause: String,
        backup: PathBuf,
        reason: String,
    },
}

pub type Result<T> = std::result::Result<T, Error>;

/// The per-user data directory: `%APPDATA%\looji` on Windows,
/// `~/.local/share/looji` on Linux. `None` if the OS reports no home directory.
pub fn data_dir() -> Option<PathBuf> {
    directories::BaseDirs::new().map(|dirs| dirs.data_dir().join("looji"))
}

pub fn database_path(data_dir: &Path) -> PathBuf {
    data_dir.join(DATABASE_FILE_NAME)
}

/// Opens the app database at `path`, creating it on first run, and migrates it to the latest schema.
pub fn open_database(path: &Path) -> Result<Connection> {
    open_database_with_migrations(path, schema::MIGRATIONS)
}

/// Like [`open_database`], but migrates to the newest entry in `migrations` instead of the app's list.
pub fn open_database_with_migrations(path: &Path, migrations: &[Migration]) -> Result<Connection> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| Error::CreateDirectory {
            path: parent.to_path_buf(),
            source,
        })?;
    }

    let mut connection = Connection::open(path).map_err(|source| Error::Open { source })?;
    configure(&connection)?;

    // An upgrade changes a database that already holds data, so it is backed up first. A first run has nothing to lose.
    let pending = migrations::pending_versions(&connection, migrations)?;
    let backup = if !pending.is_empty() && migrations::has_applied(&connection)? {
        Some(backup::create(&connection, path, &pending)?)
    } else {
        None
    };

    match migrate(&mut connection, migrations) {
        Ok(_) => {
            if let Some(backup) = &backup {
                backup::mark_pending(path, backup)?;
            }
            Ok(connection)
        }
        Err(err) => {
            let Some(backup) = backup else {
                return Err(err);
            };
            // The database is closed before the backup goes back over it.
            drop(connection);
            match backup::restore(path, &backup) {
                Ok(()) => Err(Error::UpgradeRolledBack {
                    cause: err.to_string(),
                    backup: backup.path,
                }),
                Err(restore) => Err(Error::RestoreFailed {
                    cause: err.to_string(),
                    backup: backup.path,
                    reason: restore.to_string(),
                }),
            }
        }
    }
}

/// The app's own migrations, in order. Tests use it to build a database as an earlier version left it.
pub fn app_migrations() -> &'static [Migration] {
    schema::MIGRATIONS
}

/// An in-memory database with the app's schema. For tests; nothing is saved to disk.
pub fn open_in_memory() -> Result<Connection> {
    let mut connection = Connection::open_in_memory().map_err(|source| Error::Open { source })?;
    configure(&connection)?;
    migrate_with_hook(&mut connection, schema::MIGRATIONS, |_| Ok(()))?;
    Ok(connection)
}

/// Connection settings. Set outside any transaction, where SQLite would ignore or reject them.
fn configure(connection: &Connection) -> Result<()> {
    // Foreign keys are off by default in SQLite. ON DELETE CASCADE does nothing without this.
    connection.pragma_update(None, "foreign_keys", true)?;
    connection.pragma_update_and_check(None, "journal_mode", "WAL", |_| Ok(()))?;
    // Wait for another connection's write instead of failing with SQLITE_BUSY. A slow disk, such as a CI runner's, can
    // hold a write for several seconds, so this is generous.
    connection.busy_timeout(std::time::Duration::from_secs(30))?;
    repositories::task_list::register_functions(connection)?;
    Ok(())
}

/// Starts a transaction that takes the write lock at once. A transaction that reads first and writes later can be
/// refused when another connection writes in between, and SQLite doesn't wait out the busy timeout for that. Taking
/// the lock first makes a concurrent writer wait its turn instead.
pub(crate) fn write_transaction(
    connection: &Connection,
) -> rusqlite::Result<rusqlite::Transaction<'_>> {
    rusqlite::Transaction::new_unchecked(connection, rusqlite::TransactionBehavior::Immediate)
}
