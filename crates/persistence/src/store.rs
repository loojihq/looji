use std::path::Path;

use rusqlite::Connection;

use crate::Result;

/// The app's SQLite-backed repositories. Each aggregate's trait is implemented in `repositories`.
pub struct SqliteStore {
    pub(crate) connection: Connection,
}

impl SqliteStore {
    /// Opens the app database at `path`, creating and migrating it as needed.
    pub fn open(path: &Path) -> Result<Self> {
        Ok(Self {
            connection: crate::open_database(path)?,
        })
    }

    /// An in-memory store with the app's schema. For tests.
    pub fn open_in_memory() -> Result<Self> {
        Ok(Self {
            connection: crate::open_in_memory()?,
        })
    }
}
