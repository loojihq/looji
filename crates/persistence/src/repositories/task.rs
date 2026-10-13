use looji_domain::{ProjectId, RepoResult, Task, TaskId, TaskRecord, TaskRepository, TeamMemberId};
use rusqlite::{Row, params};

use super::{
    execute, get, id_text, query_all, query_one, read_id, read_optional_id, read_position,
    read_status, read_timestamp,
};
use crate::SqliteStore;

const TASK_COLUMNS: &str = "id, project_id, title, description, status, position, \
     estimate_minutes, assignee_id, created_at, updated_at";

fn read_task(row: &Row<'_>) -> RepoResult<TaskRecord> {
    Ok(TaskRecord {
        id: read_id(row, 0, TaskId::from_uuid)?,
        project_id: read_id(row, 1, ProjectId::from_uuid)?,
        title: get(row, 2)?,
        description: get(row, 3)?,
        status: read_status(row, 4)?,
        position: read_position(row, 5)?,
        estimate_minutes: get(row, 6)?,
        assignee_id: read_optional_id(row, 7, TeamMemberId::from_uuid)?,
        created_at: read_timestamp(row, 8)?,
        updated_at: read_timestamp(row, 9)?,
    })
}

impl TaskRepository for SqliteStore {
    fn save(&self, task: &Task) -> RepoResult<()> {
        execute(
            &self.connection,
            "INSERT INTO tasks (id, project_id, title, description, status, position,
                                estimate_minutes, assignee_id, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT (id) DO UPDATE SET
                 project_id = excluded.project_id,
                 title = excluded.title,
                 description = excluded.description,
                 status = excluded.status,
                 position = excluded.position,
                 estimate_minutes = excluded.estimate_minutes,
                 assignee_id = excluded.assignee_id,
                 updated_at = excluded.updated_at",
            params![
                id_text(task.id().as_uuid()),
                id_text(task.project_id().as_uuid()),
                task.title(),
                task.description(),
                task.status().as_str(),
                task.position().as_str(),
                task.estimate()
                    .map(|estimate| i64::from(estimate.minutes())),
                task.assignee_id().map(|id| id_text(id.as_uuid())),
                task.created_at().unix_millis(),
                task.updated_at().unix_millis(),
            ],
        )
    }

    fn find(&self, id: TaskId) -> RepoResult<Option<Task>> {
        let sql = format!("SELECT {TASK_COLUMNS} FROM tasks WHERE id = ?1");
        let record = query_one(
            &self.connection,
            &sql,
            params![id_text(id.as_uuid())],
            read_task,
        )?;
        Ok(record.map(Task::try_from).transpose()?)
    }

    fn list_for_project(&self, project: ProjectId) -> RepoResult<Vec<Task>> {
        let sql = format!(
            "SELECT {TASK_COLUMNS} FROM tasks WHERE project_id = ?1 ORDER BY position COLLATE BINARY, id"
        );
        query_all(
            &self.connection,
            &sql,
            params![id_text(project.as_uuid())],
            read_task,
        )?
        .into_iter()
        .map(|record| Ok(Task::try_from(record)?))
        .collect()
    }

    fn delete(&self, id: TaskId) -> RepoResult<()> {
        execute(
            &self.connection,
            "DELETE FROM tasks WHERE id = ?1",
            params![id_text(id.as_uuid())],
        )
    }
}
