//! The history of one task: the edits recorded with each save, and the status moves recorded by
//! `move_task`. Both kinds of row are written in the same transaction as the change they describe.

use looji_domain::{RepoError, RepoResult, Task, TaskEvent, TaskEventKind, TaskId, Timestamp};
use rusqlite::{Row, params};
use uuid::Uuid;

use super::task::write_task;
use super::{id_text, query_all, read_status, read_timestamp};
use crate::SqliteStore;

fn read_edit(row: &Row<'_>) -> RepoResult<TaskEvent> {
    let kind: String = super::get(row, 0)?;
    let payload: String = super::get(row, 1)?;
    let kind = match kind.as_str() {
        "created" => TaskEventKind::Created,
        _ => TaskEventKind::Edited {
            fields: parse_fields(&payload),
        },
    };
    Ok(TaskEvent {
        at: read_timestamp(row, 2)?,
        kind,
    })
}

fn read_move(row: &Row<'_>) -> RepoResult<TaskEvent> {
    Ok(TaskEvent {
        at: read_timestamp(row, 2)?,
        kind: TaskEventKind::StatusChanged {
            from: read_status(row, 0)?,
            to: read_status(row, 1)?,
        },
    })
}

/// Reads the field names from a payload written as a JSON array of plain names.
fn parse_fields(payload: &str) -> Vec<String> {
    payload
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .split(',')
        .map(|field| field.trim().trim_matches('"').to_string())
        .filter(|field| !field.is_empty())
        .collect()
}

fn payload_for(fields: &[String]) -> String {
    let quoted: Vec<String> = fields.iter().map(|field| format!("\"{field}\"")).collect();
    format!("[{}]", quoted.join(","))
}

impl SqliteStore {
    /// Saves a task and records the edit in the same transaction. A creation or an edit is
    /// recorded here. A status move is recorded by `move_task`, so it's refused here.
    pub fn save_task_recording(&self, task: &Task, event: &TaskEventKind) -> RepoResult<()> {
        let (kind, payload) = match event {
            TaskEventKind::Created => ("created", "[]".to_string()),
            TaskEventKind::Edited { fields } => ("edited", payload_for(fields)),
            TaskEventKind::StatusChanged { .. } => {
                return Err(RepoError::Storage(
                    "status changes are recorded by moves".to_string(),
                ));
            }
        };
        let storage = |err: rusqlite::Error| RepoError::Storage(err.to_string());
        let transaction = self.connection.unchecked_transaction().map_err(storage)?;
        write_task(&transaction, task)?;
        transaction
            .execute(
                "INSERT INTO task_events (id, task_id, kind, payload, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    id_text(Uuid::new_v4()),
                    id_text(task.id().as_uuid()),
                    kind,
                    payload,
                    Timestamp::now().unix_millis(),
                ],
            )
            .map_err(storage)?;
        transaction.commit().map_err(storage)
    }

    /// Every recorded change to one task, oldest first. Status moves and edits are merged by time.
    pub fn task_history(&self, task: TaskId) -> RepoResult<Vec<TaskEvent>> {
        let key = id_text(task.as_uuid());
        let edits = query_all(
            &self.connection,
            "SELECT kind, payload, created_at FROM task_events WHERE task_id = ?1",
            params![key],
            read_edit,
        )?;
        let moves = query_all(
            &self.connection,
            "SELECT from_status, to_status, created_at FROM task_activity WHERE task_id = ?1",
            params![key],
            read_move,
        )?;
        let mut history: Vec<TaskEvent> = edits.into_iter().chain(moves).collect();
        history.sort_by_key(|event| event.at.unix_millis());
        Ok(history)
    }
}

#[cfg(test)]
mod tests {
    // Not `use super::*`: gpui's `test` macro would shadow `#[test]`.
    use super::{parse_fields, payload_for};

    #[test]
    fn edited_fields_round_trip_through_the_payload() {
        let fields = vec!["title".to_string(), "tags".to_string()];
        assert_eq!(parse_fields(&payload_for(&fields)), fields);
    }

    #[test]
    fn an_empty_payload_has_no_fields() {
        assert!(parse_fields("[]").is_empty());
    }
}
