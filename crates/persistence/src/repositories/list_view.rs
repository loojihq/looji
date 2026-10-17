use looji_domain::{ListView, ListViewRepository, ProjectId, RepoResult};
use rusqlite::params;

use super::{execute, get, id_text, query_one};
use crate::SqliteStore;

impl ListViewRepository for SqliteStore {
    /// The project's saved list settings. Stored values are checked as they're read: an unknown sort or
    /// column is dropped, and an unreadable column list falls back to all the columns.
    fn load_list_view(&self, project: ProjectId) -> RepoResult<ListView> {
        let stored = query_one(
            &self.connection,
            "SELECT sort_column, sort_direction, visible_columns FROM project_list_view
             WHERE project_id = ?1",
            params![id_text(project.as_uuid())],
            |row| {
                Ok((
                    get::<String>(row, 0)?,
                    get::<String>(row, 1)?,
                    get::<String>(row, 2)?,
                ))
            },
        )?;
        let Some((sort_column, sort_direction, columns_json)) = stored else {
            return Ok(ListView::default());
        };
        let columns: Vec<String> = serde_json::from_str(&columns_json).unwrap_or_default();
        Ok(ListView::from_stored(
            &sort_column,
            &sort_direction,
            &columns,
        ))
    }

    /// Replaces the project's list settings. A save is an upsert, so the first save creates the row.
    fn save_list_view(&self, project: ProjectId, view: &ListView) -> RepoResult<()> {
        let columns = serde_json::to_string(&view.stored_columns())
            .map_err(|err| looji_domain::RepoError::Storage(err.to_string()))?;
        execute(
            &self.connection,
            "INSERT INTO project_list_view (project_id, sort_column, sort_direction, visible_columns)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (project_id) DO UPDATE SET
                 sort_column = excluded.sort_column,
                 sort_direction = excluded.sort_direction,
                 visible_columns = excluded.visible_columns",
            params![
                id_text(project.as_uuid()),
                view.sort.column.name(),
                view.sort.direction.name(),
                columns,
            ],
        )
    }
}
