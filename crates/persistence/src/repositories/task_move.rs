//! Moving a task to another place in a status column, and reading a task's history.
//!
//! A move writes the task's status and its ordering key in one statement, and the history entry
//! in the same transaction. The key is worked out here, from the neighbours as they are in the
//! database, so a stale view in the UI can't produce a wrong key.

use looji_domain::{
    ActivityId, ActivityKind, DomainError, RepoError, RepoResult, TaskActivity, TaskId, TaskMove,
    Timestamp, key_between, needs_rebalance, rebalance,
};
use rusqlite::{OptionalExtension, Row, Transaction, params};
use thiserror::Error;
use uuid::Uuid;

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
    /// Places a task at the move's index in its destination column and records the change.
    ///
    /// A status change also writes a history entry, which is returned. A reorder within one
    /// column writes no history and returns `None`. Everything happens in one transaction, so
    /// the status, the key, and the history entry commit together or not at all.
    pub fn move_task(
        &self,
        mv: &TaskMove,
        at: Timestamp,
    ) -> Result<Option<TaskActivity>, MoveError> {
        let storage = |err: rusqlite::Error| MoveError::Storage(err.to_string());
        let task = id_text(mv.task().as_uuid());
        let project = id_text(mv.project().as_uuid());
        let to = mv.to().as_str();
        // Dropping the transaction without committing rolls back, so every early return below
        // leaves the database as it was.
        let transaction = self.connection.unchecked_transaction().map_err(storage)?;

        let current: Option<(String, String)> = transaction
            .query_row(
                "SELECT status, project_id FROM tasks WHERE id = ?1",
                params![task],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(storage)?;
        match current {
            None => return Err(MoveError::NotFound),
            Some((status, project_id)) if status != mv.from().as_str() || project_id != project => {
                return Err(MoveError::Conflict);
            }
            Some(_) => {}
        }

        let others = count_others(&transaction, &project, to, &task).map_err(storage)?;
        let index = mv.index().min(others);
        let (lower, upper) =
            neighbours(&transaction, &project, to, &task, index).map_err(storage)?;
        let key = key_between(lower.as_deref(), upper.as_deref())?;

        let key = if needs_rebalance(&[key.as_str()]) {
            renumber(&transaction, &project, to, &task, index)?
        } else {
            key
        };

        transaction
            .execute(
                "UPDATE tasks SET status = ?1, position = ?2, updated_at = ?3 WHERE id = ?4",
                params![to, key, at.unix_millis(), task],
            )
            .map_err(storage)?;

        let activity = (mv.from() != mv.to()).then(|| TaskActivity {
            id: ActivityId::new(),
            task: mv.task(),
            kind: ActivityKind::StatusChanged {
                from: mv.from(),
                to: mv.to(),
            },
            at,
        });
        if let Some(activity) = &activity {
            transaction
                .execute(
                    "INSERT INTO task_activity (id, task_id, kind, from_status, to_status, created_at)
                     VALUES (?1, ?2, 'status_changed', ?3, ?4, ?5)",
                    params![
                        id_text(activity.id.as_uuid()),
                        task,
                        mv.from().as_str(),
                        to,
                        at.unix_millis(),
                    ],
                )
                .map_err(storage)?;
        }
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

/// How many tasks other than `moved` are in the column.
fn count_others(
    transaction: &Transaction<'_>,
    project: &str,
    status: &str,
    moved: &str,
) -> rusqlite::Result<usize> {
    let count: i64 = transaction.query_row(
        "SELECT COUNT(*) FROM tasks WHERE project_id = ?1 AND status = ?2 AND id != ?3",
        params![project, status, moved],
        |row| row.get(0),
    )?;
    Ok(count as usize)
}

/// The keys either side of `index` among the column's other tasks, in key order. A key is `None`
/// at either end of the column.
fn neighbours(
    transaction: &Transaction<'_>,
    project: &str,
    status: &str,
    moved: &str,
    index: usize,
) -> rusqlite::Result<(Option<String>, Option<String>)> {
    // Read from one step before the index, so both neighbours come back in a single query.
    let start = index.saturating_sub(1) as i64;
    let mut statement = transaction.prepare(
        "SELECT position FROM tasks WHERE project_id = ?1 AND status = ?2 AND id != ?3
         ORDER BY position COLLATE BINARY, id LIMIT 2 OFFSET ?4",
    )?;
    let keys: Vec<String> = statement
        .query_map(params![project, status, moved, start], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(if index == 0 {
        (None, keys.first().cloned())
    } else {
        (keys.first().cloned(), keys.get(1).cloned())
    })
}

/// Renumbers the destination column, with `moved` placed at `index`, and returns the key for
/// `moved`. Runs inside the move's transaction, so the column is never left half-renumbered.
fn renumber(
    transaction: &Transaction<'_>,
    project: &str,
    status: &str,
    moved: &str,
    index: usize,
) -> Result<String, MoveError> {
    let storage = |err: rusqlite::Error| MoveError::Storage(err.to_string());
    let mut ids: Vec<String> = query_all_text(
        transaction,
        "SELECT id FROM tasks WHERE project_id = ?1 AND status = ?2 AND id != ?3
         ORDER BY position COLLATE BINARY, id",
        params![project, status, moved],
    )
    .map_err(storage)?;
    ids.insert(index.min(ids.len()), moved.to_string());
    let ids: Vec<TaskId> = ids
        .iter()
        .map(|text| {
            Uuid::parse_str(text)
                .map(TaskId::from_uuid)
                .map_err(|err| MoveError::Storage(format!("a stored id is invalid: {err}")))
        })
        .collect::<Result<_, _>>()?;
    let keys = rebalance(&ids);

    // Park every key in the column first, so the unique index never sees two rows share one
    // mid-way. `moved` is included when the move stays in this column, since its old key is
    // in the same index.
    transaction
        .execute(
            "UPDATE tasks SET position = 'parked:' || id WHERE project_id = ?1 AND status = ?2",
            params![project, status],
        )
        .map_err(storage)?;

    let mut moved_key = None;
    for (id, key) in &keys {
        if id_text(id.as_uuid()) == moved {
            moved_key = Some(key.clone());
            continue;
        }
        transaction
            .execute(
                "UPDATE tasks SET position = ?1 WHERE id = ?2",
                params![key, id_text(id.as_uuid())],
            )
            .map_err(storage)?;
    }
    moved_key.ok_or_else(|| MoveError::Storage("the moved task was not renumbered".to_string()))
}

fn query_all_text(
    transaction: &Transaction<'_>,
    sql: &str,
    params: impl rusqlite::Params,
) -> rusqlite::Result<Vec<String>> {
    let mut statement = transaction.prepare(sql)?;
    statement.query_map(params, |row| row.get(0))?.collect()
}
