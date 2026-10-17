//! Showing and hiding optional status columns, with the tasks in them, in one transaction.

use looji_domain::{
    DomainError, ProjectId, Status, check_hideable, resolve_visible_status, visible_statuses,
};
use rusqlite::params;
use thiserror::Error;

use crate::SqliteStore;

#[derive(Debug, Error)]
pub enum VisibilityError {
    #[error(transparent)]
    Domain(#[from] DomainError),
    #[error("that project no longer exists")]
    ProjectNotFound,
    #[error("the column could not be changed: {0}")]
    Storage(String),
}

impl SqliteStore {
    /// Shows or hides an optional status column for one project. Returns how many tasks moved.
    ///
    /// Hiding moves that status's tasks to the nearest visible status and records the origin in
    /// `hidden_from`. Showing again moves back only the tasks still at that fallback. Tasks the
    /// user has moved since are left where they are. Either way the flag and the task rows change
    /// together, so a failure leaves everything as it was.
    ///
    /// The four defaults can't be hidden, so hiding one fails. Showing one is a no-op.
    pub fn set_status_visible(
        &self,
        project: ProjectId,
        status: Status,
        visible: bool,
    ) -> Result<usize, VisibilityError> {
        if status.is_default() {
            if visible {
                return Ok(0);
            }
            check_hideable(status)?;
        }

        let storage = |err: rusqlite::Error| VisibilityError::Storage(err.to_string());
        let key = project.as_uuid().to_string();
        let origin = status.as_str();
        // Hiding and showing use the same fallback, so restoring finds the rows hiding moved.
        let fallback = resolve_visible_status(status, &visible_statuses(&[status])).as_str();

        let transaction = crate::write_transaction(&self.connection).map_err(storage)?;

        let exists: bool = transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM projects WHERE id = ?1)",
                params![key],
                |row| row.get(0),
            )
            .map_err(storage)?;
        if !exists {
            return Err(VisibilityError::ProjectNotFound);
        }

        let moved = if visible {
            let restored = transaction
                .execute(
                    "UPDATE tasks SET status = ?1, hidden_from = NULL
                     WHERE project_id = ?2 AND hidden_from = ?1 AND status = ?3",
                    params![origin, key, fallback],
                )
                .map_err(storage)?;
            // Anything still marked was moved by a hide and then changed by the user. Clear the
            // marker so it can't be restored later by mistake.
            transaction
                .execute(
                    "UPDATE tasks SET hidden_from = NULL
                     WHERE project_id = ?1 AND hidden_from IS NOT NULL",
                    params![key],
                )
                .map_err(storage)?;
            restored
        } else {
            transaction
                .execute(
                    "UPDATE tasks SET status = ?2, hidden_from = ?1
                     WHERE project_id = ?3 AND status = ?1",
                    params![origin, fallback, key],
                )
                .map_err(storage)?
        };

        transaction
            .execute(
                "UPDATE projects SET show_in_review = ?1 WHERE id = ?2",
                params![visible, key],
            )
            .map_err(storage)?;

        transaction.commit().map_err(storage)?;
        Ok(moved)
    }
}
