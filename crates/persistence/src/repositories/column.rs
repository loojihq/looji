use looji_domain::{Column, ColumnId, ColumnRecord, ColumnRepository, ProjectId, RepoResult};
use rusqlite::{Row, params};

use super::{execute, get, id_text, query_all, read_id, read_position, read_status};
use crate::SqliteStore;

fn read_column(row: &Row<'_>) -> RepoResult<ColumnRecord> {
    Ok(ColumnRecord {
        id: read_id(row, 0, ColumnId::from_uuid)?,
        project_id: read_id(row, 1, ProjectId::from_uuid)?,
        status: read_status(row, 2)?,
        name: get(row, 3)?,
        position: read_position(row, 4)?,
        visible: get(row, 5)?,
    })
}

impl ColumnRepository for SqliteStore {
    fn save(&self, column: &Column) -> RepoResult<()> {
        execute(
            &self.connection,
            "INSERT INTO board_columns (id, project_id, status, name, position, visible)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT (id) DO UPDATE SET
                 project_id = excluded.project_id,
                 status = excluded.status,
                 name = excluded.name,
                 position = excluded.position,
                 visible = excluded.visible",
            params![
                id_text(column.id().as_uuid()),
                id_text(column.project_id().as_uuid()),
                column.status().as_str(),
                column.name(),
                column.position().as_str(),
                column.is_visible(),
            ],
        )
    }

    fn list_for_project(&self, project: ProjectId) -> RepoResult<Vec<Column>> {
        query_all(
            &self.connection,
            "SELECT id, project_id, status, name, position, visible FROM board_columns
             WHERE project_id = ?1
             ORDER BY position, id",
            params![id_text(project.as_uuid())],
            read_column,
        )?
        .into_iter()
        .map(|record| Ok(Column::try_from(record)?))
        .collect()
    }

    fn delete(&self, id: ColumnId) -> RepoResult<()> {
        execute(
            &self.connection,
            "DELETE FROM board_columns WHERE id = ?1",
            params![id_text(id.as_uuid())],
        )
    }
}
