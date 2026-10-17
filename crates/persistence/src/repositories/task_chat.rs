//! Conversations about a task and the review notes made for it. Both belong to their task, so deleting the task
//! removes them. Messages and notes keep their order by a position column rather than by time, so two written in
//! the same millisecond still come back in the order they were written.

use looji_domain::{
    ChatMessage, ChatThreadId, MessageRole, NoteKind, ReplyState, RepoError, RepoResult,
    ReviewNote, ReviewNoteRepository, TaskChatRepository, TaskId, Timestamp,
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
    ) -> RepoResult<u32> {
        let transaction = crate::write_transaction(&self.connection).map_err(storage)?;
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
        transaction.commit().map_err(storage)?;
        u32::try_from(position).map_err(|_| RepoError::Storage("too many messages".to_string()))
    }

    /// Writes the reply's final text and how it ended. A stopped or failed reply keeps the text it had.
    fn finish_message(
        &self,
        thread: ChatThreadId,
        position: u32,
        content: &str,
        state: &ReplyState,
    ) -> RepoResult<()> {
        let error = match state {
            ReplyState::Failed { reason } => Some(reason.clone()),
            _ => None,
        };
        execute(
            &self.connection,
            "UPDATE task_messages SET content = ?3, state = ?4, error = ?5
             WHERE thread_id = ?1 AND position = ?2",
            params![
                id_text(thread.as_uuid()),
                position,
                content,
                state.label(),
                error,
            ],
        )
    }

    /// Removes the message at `position` and every message after it. Used to retry a reply: the failed one goes.
    fn truncate_from(&self, thread: ChatThreadId, position: u32) -> RepoResult<()> {
        execute(
            &self.connection,
            "DELETE FROM task_messages WHERE thread_id = ?1 AND position >= ?2",
            params![id_text(thread.as_uuid()), position],
        )
    }

    fn messages(&self, thread: ChatThreadId) -> RepoResult<Vec<ChatMessage>> {
        query_all(
            &self.connection,
            "SELECT position, role, content, model, state, error, created_at FROM task_messages
             WHERE thread_id = ?1 ORDER BY position",
            params![id_text(thread.as_uuid())],
            |row| {
                let role: String = get(row, 1)?;
                let state: String = get(row, 4)?;
                let error: Option<String> = get(row, 5)?;
                Ok(ChatMessage {
                    position: get(row, 0)?,
                    role: MessageRole::parse(&role).ok_or_else(|| {
                        RepoError::Storage(format!("unknown message role {role:?}"))
                    })?,
                    content: get(row, 2)?,
                    model: get(row, 3)?,
                    state: match state.as_str() {
                        "stopped" => ReplyState::Stopped,
                        "failed" => ReplyState::Failed {
                            reason: error.unwrap_or_else(|| "the reply failed".to_string()),
                        },
                        _ => ReplyState::Complete,
                    },
                    created_at: read_timestamp(row, 6)?,
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
        let transaction = crate::write_transaction(&self.connection).map_err(storage)?;
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
