//! The reviews a task has had. Each is kept, so the panel can show how a task's reviews went over time.

use looji_domain::{RepoResult, ReviewHistoryRepository, ReviewRecord, TaskId};
use rusqlite::params;

use super::{get, id_text, query_all, read_timestamp, storage};
use crate::SqliteStore;

impl ReviewHistoryRepository for SqliteStore {
    fn add_review(&self, task: TaskId, record: &ReviewRecord) -> RepoResult<()> {
        self.connection
            .execute(
                "INSERT INTO task_reviews (task_id, outcome, summary, review_json, model, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    id_text(task.as_uuid()),
                    record.outcome,
                    record.summary,
                    record.review_json,
                    record.model,
                    record.created_at.unix_millis(),
                ],
            )
            .map_err(storage)?;
        Ok(())
    }

    fn reviews(&self, task: TaskId) -> RepoResult<Vec<ReviewRecord>> {
        query_all(
            &self.connection,
            "SELECT outcome, summary, review_json, model, created_at FROM task_reviews
             WHERE task_id = ?1 ORDER BY created_at DESC, id DESC",
            params![id_text(task.as_uuid())],
            |row| {
                Ok(ReviewRecord {
                    outcome: get(row, 0)?,
                    summary: get(row, 1)?,
                    review_json: get(row, 2)?,
                    model: get(row, 3)?,
                    created_at: read_timestamp(row, 4)?,
                })
            },
        )
    }
}
