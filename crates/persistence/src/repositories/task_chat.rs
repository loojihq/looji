//! Conversations about a task and the review notes made for it. Both belong to their task, so deleting the task
//! removes them. Messages and notes keep their order by a position column rather than by time, so two written in
//! the same millisecond still come back in the order they were written.

use looji_domain::{
    ChatMessage, ChatThreadId, MessageRole, NoteKind, RepoError, RepoResult, ReviewNote,
    ReviewNoteRepository, TaskChatRepository, TaskId, Timestamp,
};
use rusqlite::params;
use uuid::Uuid;

use super::{execute, get, id_text, query_all, read_id, read_timestamp, storage};
use crate::SqliteStore;

impl TaskChatRepository for SqliteStore {
    fn start_thread(&self, task: TaskId) -> RepoResult<ChatThreadId> {
        let thread = ChatThreadId::new();
        execute(
            &self.connection,
            "INSERT INTO task_threads (id, task_id, created_at) VALUES (?1, ?2, ?3)",
            params![
                id_text(thread.as_uuid()),
                id_text(task.as_uuid()),
                Timestamp::now().unix_millis(),
            ],
        )?;
        Ok(thread)
    }

    fn threads(&self, task: TaskId) -> RepoResult<Vec<ChatThreadId>> {
        query_all(
            &self.connection,
            "SELECT id FROM task_threads WHERE task_id = ?1 ORDER BY created_at, id",
            params![id_text(task.as_uuid())],
            |row| read_id(row, 0, ChatThreadId::from_uuid),
        )
    }

    /// Appends at the next position in the thread, in one transaction, so two writers can't take the same one.
    fn append_message(
        &self,
        thread: ChatThreadId,
        role: MessageRole,
        content: &str,
        model: Option<&str>,
    ) -> RepoResult<()> {
        let transaction = self.connection.unchecked_transaction().map_err(storage)?;
        let key = id_text(thread.as_uuid());
        let position: i64 = transaction
            .query_row(
                "SELECT COALESCE(MAX(position) + 1, 0) FROM task_messages WHERE thread_id = ?1",
                params![key],
                |row| row.get(0),
            )
            .map_err(storage)?;
        transaction
            .execute(
                "INSERT INTO task_messages (id, thread_id, position, role, content, model, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    id_text(Uuid::new_v4()),
                    key,
                    position,
                    role.label(),
                    content,
                    model,
                    Timestamp::now().unix_millis(),
                ],
            )
            .map_err(storage)?;
        transaction.commit().map_err(storage)
    }

    fn messages(&self, thread: ChatThreadId) -> RepoResult<Vec<ChatMessage>> {
        query_all(
            &self.connection,
            "SELECT role, content, model, created_at FROM task_messages
             WHERE thread_id = ?1 ORDER BY position",
            params![id_text(thread.as_uuid())],
            |row| {
                let role: String = get(row, 0)?;
                Ok(ChatMessage {
                    role: MessageRole::parse(&role).ok_or_else(|| {
                        RepoError::Storage(format!("unknown message role {role:?}"))
                    })?,
                    content: get(row, 1)?,
                    model: get(row, 2)?,
                    created_at: read_timestamp(row, 3)?,
                })
            },
        )
    }
}

impl ReviewNoteRepository for SqliteStore {
    fn replace_notes(
        &self,
        task: TaskId,
        notes: &[ReviewNote],
        model: Option<&str>,
    ) -> RepoResult<()> {
        let transaction = self.connection.unchecked_transaction().map_err(storage)?;
        let key = id_text(task.as_uuid());
        transaction
            .execute(
                "DELETE FROM task_review_notes WHERE task_id = ?1",
                params![key],
            )
            .map_err(storage)?;
        let now = Timestamp::now().unix_millis();
        for (position, note) in notes.iter().enumerate() {
            let files = serde_json::to_string(&note.files)
                .map_err(|err| RepoError::Storage(err.to_string()))?;
            transaction
                .execute(
                    "INSERT INTO task_review_notes (task_id, position, kind, body, files, model, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![key, position as i64, note.kind.label(), note.body, files, model, now],
                )
                .map_err(storage)?;
        }
        transaction.commit().map_err(storage)
    }

    fn notes(&self, task: TaskId) -> RepoResult<Vec<ReviewNote>> {
        query_all(
            &self.connection,
            "SELECT kind, body, files FROM task_review_notes WHERE task_id = ?1 ORDER BY position",
            params![id_text(task.as_uuid())],
            |row| {
                let kind: String = get(row, 0)?;
                let files: String = get(row, 2)?;
                Ok(ReviewNote {
                    kind: NoteKind::parse(&kind)
                        .ok_or_else(|| RepoError::Storage(format!("unknown note kind {kind:?}")))?,
                    body: get(row, 1)?,
                    files: serde_json::from_str(&files)
                        .map_err(|err| RepoError::Storage(err.to_string()))?,
                })
            },
        )
    }
}
