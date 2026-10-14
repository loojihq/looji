use std::collections::HashMap;

use looji_domain::{
    DueDate, Priority, ProjectId, RepoResult, Task, TaskId, TaskRecord, TaskRepository,
    TeamMemberId,
};
use rusqlite::{Connection, Row, Transaction, params};

use super::{
    execute, get, id_text, query_all, query_one, read_id, read_optional_id, read_position,
    read_status, read_timestamp, storage,
};
use crate::SqliteStore;

const TASK_COLUMNS: &str = "id, project_id, title, description, status, position, \
     estimate_minutes, assignee_id, created_at, updated_at, priority, due_on";

fn read_task(row: &Row<'_>) -> RepoResult<TaskRecord> {
    let priority: Option<String> = get(row, 10)?;
    let due_on: Option<String> = get(row, 11)?;
    Ok(TaskRecord {
        id: read_id(row, 0, TaskId::from_uuid)?,
        project_id: read_id(row, 1, ProjectId::from_uuid)?,
        title: get(row, 2)?,
        description: get(row, 3)?,
        status: read_status(row, 4)?,
        position: read_position(row, 5)?,
        estimate_minutes: get(row, 6)?,
        assignee_id: read_optional_id(row, 7, TeamMemberId::from_uuid)?,
        priority: priority.map(|text| text.parse::<Priority>()).transpose()?,
        due_on: due_on.map(|text| text.parse::<DueDate>()).transpose()?,
        // Filled in from the tags table by the caller.
        tags: Vec::new(),
        created_at: read_timestamp(row, 8)?,
        updated_at: read_timestamp(row, 9)?,
    })
}

/// The tags of the tasks a query selects, keyed by task id. `sql` selects `task_id, tag`.
fn load_tags(
    connection: &Connection,
    sql: &str,
    params: impl rusqlite::Params,
) -> RepoResult<HashMap<String, Vec<String>>> {
    let rows = query_all(connection, sql, params, |row| {
        Ok((get::<String>(row, 0)?, get::<String>(row, 1)?))
    })?;
    let mut tags: HashMap<String, Vec<String>> = HashMap::new();
    for (task, tag) in rows {
        tags.entry(task).or_default().push(tag);
    }
    Ok(tags)
}

impl TaskRepository for SqliteStore {
    /// Writes the task and replaces its tags in one transaction, so a task never has half its tags.
    fn save(&self, task: &Task) -> RepoResult<()> {
        let transaction = self.connection.unchecked_transaction().map_err(storage)?;
        write_task(&transaction, task)?;
        transaction.commit().map_err(storage)
    }

    fn find(&self, id: TaskId) -> RepoResult<Option<Task>> {
        let sql = format!("SELECT {TASK_COLUMNS} FROM tasks WHERE id = ?1");
        let key = id_text(id.as_uuid());
        let Some(mut record) = query_one(&self.connection, &sql, params![key], read_task)? else {
            return Ok(None);
        };
        let mut tags = load_tags(
            &self.connection,
            "SELECT task_id, tag FROM task_tags WHERE task_id = ?1 ORDER BY tag",
            params![key],
        )?;
        record.tags = tags.remove(&key).unwrap_or_default();
        Ok(Some(Task::try_from(record)?))
    }

    fn list_for_project(&self, project: ProjectId) -> RepoResult<Vec<Task>> {
        let sql = format!(
            "SELECT {TASK_COLUMNS} FROM tasks WHERE project_id = ?1 ORDER BY position COLLATE BINARY, id"
        );
        let project_key = id_text(project.as_uuid());
        let mut records = query_all(&self.connection, &sql, params![project_key], read_task)?;
        let mut tags = load_tags(
            &self.connection,
            "SELECT task_id, tag FROM task_tags
             WHERE task_id IN (SELECT id FROM tasks WHERE project_id = ?1)
             ORDER BY tag",
            params![project_key],
        )?;
        for record in &mut records {
            record.tags = tags
                .remove(&id_text(record.id.as_uuid()))
                .unwrap_or_default();
        }
        records
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

/// Writes a task and its tags inside a transaction the caller owns. Used by [`TaskRepository::save`]
/// and by the writes that also record history, so both commit together.
pub(super) fn write_task(transaction: &Transaction<'_>, task: &Task) -> RepoResult<()> {
    let id = id_text(task.id().as_uuid());
    transaction
        .execute(
            "INSERT INTO tasks (id, project_id, title, description, status, position,
                                    estimate_minutes, assignee_id, priority, due_on,
                                    created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
                 ON CONFLICT (id) DO UPDATE SET
                     project_id = excluded.project_id,
                     title = excluded.title,
                     description = excluded.description,
                     status = excluded.status,
                     position = excluded.position,
                     estimate_minutes = excluded.estimate_minutes,
                     assignee_id = excluded.assignee_id,
                     priority = excluded.priority,
                     due_on = excluded.due_on,
                     updated_at = excluded.updated_at",
            params![
                id,
                id_text(task.project_id().as_uuid()),
                task.title(),
                task.description(),
                task.status().as_str(),
                task.position().as_str(),
                task.estimate()
                    .map(|estimate| i64::from(estimate.minutes())),
                task.assignee_id().map(|id| id_text(id.as_uuid())),
                task.priority().map(Priority::as_str),
                task.due_on().map(|due| due.to_string()),
                task.created_at().unix_millis(),
                task.updated_at().unix_millis(),
            ],
        )
        .map_err(storage)?;
    transaction
        .execute("DELETE FROM task_tags WHERE task_id = ?1", params![id])
        .map_err(storage)?;
    for tag in task.tags() {
        transaction
            .execute(
                "INSERT INTO task_tags (task_id, tag) VALUES (?1, ?2)",
                params![id, tag],
            )
            .map_err(storage)?;
    }
    Ok(())
}
