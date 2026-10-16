use std::path::Path;

use looji_domain::{Timestamp, WorkspaceId};
use rusqlite::{Connection, OptionalExtension, params};

use crate::{
    Result,
    icons::{self, IconError},
};

/// The app's SQLite-backed repositories. Each aggregate's trait is implemented in `repositories`.
pub struct SqliteStore {
    pub(crate) connection: Connection,
}

impl SqliteStore {
    /// Opens the app database at `path`, creating and migrating it as needed.
    pub fn open(path: &Path) -> Result<Self> {
        let connection = crate::open_database(path)?;
        // A generation runs in the process that started it. One still generating when the store opens was cut
        // off when the app closed, so it is interrupted now, and its text is kept.
        crate::repositories::plan_draft::interrupt_generating(&connection, Timestamp::now())?;
        Ok(Self { connection })
    }

    /// An in-memory store with the app's schema. For tests.
    pub fn open_in_memory() -> Result<Self> {
        Ok(Self {
            connection: crate::open_in_memory()?,
        })
    }

    /// Points a workspace at `new_file`, or clears its icon when `None`.
    ///
    /// The database row changes in one transaction. If that fails, `new_file` is deleted, so the
    /// old icon stays in place. Once the change is committed, the previous icon file is deleted.
    /// Deleting an old file is best-effort: a leftover file is harmless, so its error is ignored.
    pub fn replace_workspace_icon(
        &self,
        icons_dir: &Path,
        workspace: WorkspaceId,
        new_file: Option<&str>,
    ) -> std::result::Result<(), IconError> {
        match self.update_icon_column(workspace, new_file) {
            Ok(previous) => {
                if let Some(old) = previous.filter(|old| Some(old.as_str()) != new_file) {
                    let _ = icons::remove_icon(icons_dir, &old);
                }
                Ok(())
            }
            Err(err) => {
                if let Some(new) = new_file {
                    let _ = icons::remove_icon(icons_dir, new);
                }
                Err(err)
            }
        }
    }

    /// Writes the icon column in a transaction and returns the icon it replaced.
    fn update_icon_column(
        &self,
        workspace: WorkspaceId,
        new_file: Option<&str>,
    ) -> std::result::Result<Option<String>, IconError> {
        let id = workspace.as_uuid().to_string();
        let database = |err: rusqlite::Error| IconError::Database(err.to_string());

        let transaction = self.connection.unchecked_transaction().map_err(database)?;
        let previous: Option<Option<String>> = transaction
            .query_row(
                "SELECT icon_file FROM workspaces WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .optional()
            .map_err(database)?;
        let previous = previous.ok_or(IconError::WorkspaceNotFound)?;

        transaction
            .execute(
                "UPDATE workspaces SET icon_file = ?1 WHERE id = ?2",
                params![new_file, id],
            )
            .map_err(database)?;
        transaction.commit().map_err(database)?;
        Ok(previous)
    }
}
