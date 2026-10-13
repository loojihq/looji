//! SQLite storage: where the database lives and how its schema is set up.

mod migrations;

use std::{
    fs,
    path::{Path, PathBuf},
};

pub use rusqlite::Connection;
use thiserror::Error;

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
        path: PathBuf,
        #[source]
        source: rusqlite::Error,
    },
    #[error("could not set up the database schema: {0}")]
    Schema(#[from] rusqlite::Error),
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

/// Opens the database at `path`, creating the file and its parent directories
/// on first run, then brings the schema up to date.
pub fn open_database(path: &Path) -> Result<Connection> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| Error::CreateDirectory {
            path: parent.to_path_buf(),
            source,
        })?;
    }

    let mut connection = Connection::open(path).map_err(|source| Error::Open {
        path: path.to_path_buf(),
        source,
    })?;
    migrations::apply(&mut connection)?;
    Ok(connection)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_missing_directories_and_database_on_first_run() {
        let base = std::env::temp_dir().join(format!("looji-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let path = database_path(&base.join("nested").join("data"));

        let connection = open_database(&path).expect("database opens on first run");
        assert!(path.exists());
        drop(connection);

        // Reopening an existing database must also succeed.
        open_database(&path).expect("database reopens on later runs");

        fs::remove_dir_all(&base).expect("cleans up temp directory");
    }
}
