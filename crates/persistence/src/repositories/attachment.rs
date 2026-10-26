use looji_domain::{
    Attachment, AttachmentRepository, DomainError, MediaKind, RepoResult, TaskId, Timestamp,
};
use rusqlite::{Row, params};
use uuid::Uuid;

use super::{execute, get, query_all};
use crate::SqliteStore;

fn read(row: &Row<'_>) -> RepoResult<Attachment> {
    let task: String = get(row, 0)?;
    let kind: String = get(row, 3)?;
    Ok(Attachment {
        task: Uuid::parse_str(&task)
            .map(TaskId::from_uuid)
            .map_err(|_| DomainError::InvalidId(task))?,
        stored_name: get(row, 1)?,
        file_name: get(row, 2)?,
        kind: MediaKind::from_stored(&kind).ok_or(DomainError::InvalidId(kind))?,
        created_at: Timestamp::from_unix_millis(get(row, 4)?),
    })
}

impl AttachmentRepository for SqliteStore {
    fn list_for_task(&self, task: TaskId) -> RepoResult<Vec<Attachment>> {
        query_all(
            &self.connection,
            "SELECT task_id, stored_name, file_name, kind, created_at
             FROM task_attachments WHERE task_id = ?1
             ORDER BY created_at, stored_name",
            params![task.as_uuid().to_string()],
            read,
        )
    }

    fn add(&self, attachment: &Attachment) -> RepoResult<()> {
        execute(
            &self.connection,
            "INSERT INTO task_attachments (task_id, stored_name, file_name, kind, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                attachment.task.as_uuid().to_string(),
                attachment.stored_name,
                attachment.file_name,
                attachment.kind.stored(),
                attachment.created_at.unix_millis(),
            ],
        )
    }

    fn remove(&self, task: TaskId, stored_name: &str) -> RepoResult<()> {
        execute(
            &self.connection,
            "DELETE FROM task_attachments WHERE task_id = ?1 AND stored_name = ?2",
            params![task.as_uuid().to_string(), stored_name],
        )
    }
}
