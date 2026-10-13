//! Moving a task to another status, and reading its history. The status change and the history
//! entry are written in one transaction, so they commit together or not at all.

use looji_domain::{
    ActivityId, ActivityKind, DomainError, RepoError, RepoResult, TaskActivity, TaskId, TaskMove,
    Timestamp,
};
use rusqlite::{Row, params};
use thiserror::Error;

use super::{id_text, query_all, read_id, read_status, read_timestamp};
use crate::SqliteStore;

#[derive(Debug, Error)]
pub enum MoveError {
    #[error(transparent)]
    Domain(#[from] DomainError),
    /// The task isn't in the status the move started from, so it changed since it was loaded.
    #[error("the task changed before the move was saved")]
    Conflict,
    #[error("that task no longer exists")]
    NotFound,
    #[error("the move could not be saved: {0}")]
    Storage(String),
}

fn read_activity(row: &Row<'_>) -> RepoResult<TaskActivity> {
    let from = read_status(row, 2)?;
    let to = read_status(row, 3)?;
    Ok(TaskActivity {
        id: read_id(row, 0, ActivityId::from_uuid)?,
        task: read_id(row, 1, TaskId::from_uuid)?,
        kind: ActivityKind::StatusChanged { from, to },
        at: read_timestamp(row, 4)?,
    })
}

impl SqliteStore {
    /// Moves a task to the target status and records the change, in one transaction. Returns the
    /// history entry it wrote.
    ///
    /// The update only applies while the task is still in the status the move started from. If
    /// it isn't, nothing is written and the result is [`MoveError::Conflict`].
    pub fn move_task(&self, mv: &TaskMove, at: Timestamp) -> Result<TaskActivity, MoveError> {
        let storage = |err: rusqlite::Error| MoveError::Storage(err.to_string());
        let task = id_text(mv.task().as_uuid());
        // Dropping the transaction without committing rolls back, so every early return below
        // leaves the database as it was.
        let transaction = self.connection.unchecked_transaction().map_err(storage)?;

        let changed = transaction
            .execute(
                "UPDATE tasks SET status = ?1, updated_at = ?2
                 WHERE id = ?3 AND project_id = ?4 AND status = ?5",
                params![
                    mv.to().as_str(),
                    at.unix_millis(),
                    task,
                    id_text(mv.project().as_uuid()),
                    mv.from().as_str(),
                ],
            )
            .map_err(storage)?;
        if changed == 0 {
            let exists: bool = transaction
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM tasks WHERE id = ?1)",
                    params![task],
                    |row| row.get(0),
                )
                .map_err(storage)?;
            return Err(if exists {
                MoveError::Conflict
            } else {
                MoveError::NotFound
            });
        }

        let activity = TaskActivity {
            id: ActivityId::new(),
            task: mv.task(),
            kind: ActivityKind::StatusChanged {
                from: mv.from(),
                to: mv.to(),
            },
            at,
        };
        transaction
            .execute(
                "INSERT INTO task_activity (id, task_id, kind, from_status, to_status, created_at)
                 VALUES (?1, ?2, 'status_changed', ?3, ?4, ?5)",
                params![
                    id_text(activity.id.as_uuid()),
                    task,
                    mv.from().as_str(),
                    mv.to().as_str(),
                    at.unix_millis(),
                ],
            )
            .map_err(storage)?;
        transaction.commit().map_err(storage)?;
        Ok(activity)
    }

    /// The history of one task, oldest first.
    pub fn task_activity(&self, task: TaskId) -> Result<Vec<TaskActivity>, MoveError> {
        query_all(
            &self.connection,
            "SELECT id, task_id, from_status, to_status, created_at FROM task_activity
             WHERE task_id = ?1 ORDER BY created_at, rowid",
            params![id_text(task.as_uuid())],
            read_activity,
        )
        .map_err(|err: RepoError| MoveError::Storage(err.to_string()))
    }
}
