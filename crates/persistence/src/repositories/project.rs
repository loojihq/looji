use looji_domain::{
    Project, ProjectId, ProjectRecord, ProjectRepository, RepoError, RepoResult, WorkspaceId,
};
use rusqlite::{Row, params};

use super::{execute, get, id_text, query_all, query_one, read_id, read_timestamp, storage};
use crate::SqliteStore;

const NAME_INDEX: &str = "projects_name_unique";

fn read_project(row: &Row<'_>) -> RepoResult<ProjectRecord> {
    Ok(ProjectRecord {
        id: read_id(row, 0, ProjectId::from_uuid)?,
        workspace_id: read_id(row, 1, WorkspaceId::from_uuid)?,
        name: get(row, 2)?,
        description: get(row, 3)?,
        show_in_review: get(row, 4)?,
        created_at: read_timestamp(row, 5)?,
        updated_at: read_timestamp(row, 6)?,
    })
}

/// Turns a write rejected by the name index into [`RepoError::NameTaken`].
fn save_error(err: rusqlite::Error) -> RepoError {
    if err.to_string().contains(NAME_INDEX) {
        RepoError::NameTaken
    } else {
        storage(err)
    }
}

const PROJECT_COLUMNS: &str =
    "id, workspace_id, name, description, show_in_review, created_at, updated_at";

impl ProjectRepository for SqliteStore {
    /// Inserts or updates the name and description. The column flags are written only on insert:
    /// later changes go through `SqliteStore::set_status_visible`, so a rename can't undo them.
    fn save(&self, project: &Project) -> RepoResult<()> {
        self.connection
            .execute(
                "INSERT INTO projects (id, workspace_id, name, description, show_in_review, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT (id) DO UPDATE SET
                     name = excluded.name,
                     description = excluded.description,
                     updated_at = excluded.updated_at",
                params![
                    id_text(project.id().as_uuid()),
                    id_text(project.workspace_id().as_uuid()),
                    project.name(),
                    project.description(),
                    project.show_in_review(),
                    project.created_at().unix_millis(),
                    project.updated_at().unix_millis(),
                ],
            )
            .map_err(save_error)?;
        Ok(())
    }

    fn find(&self, workspace: WorkspaceId, id: ProjectId) -> RepoResult<Option<Project>> {
        let sql =
            format!("SELECT {PROJECT_COLUMNS} FROM projects WHERE id = ?1 AND workspace_id = ?2");
        let record = query_one(
            &self.connection,
            &sql,
            params![id_text(id.as_uuid()), id_text(workspace.as_uuid())],
            read_project,
        )?;
        Ok(record.map(Project::try_from).transpose()?)
    }

    fn list_for_workspace(&self, workspace: WorkspaceId) -> RepoResult<Vec<Project>> {
        let sql = format!(
            "SELECT {PROJECT_COLUMNS} FROM projects WHERE workspace_id = ?1 ORDER BY created_at, id"
        );
        query_all(
            &self.connection,
            &sql,
            params![id_text(workspace.as_uuid())],
            read_project,
        )?
        .into_iter()
        .map(|record| Ok(Project::try_from(record)?))
        .collect()
    }

    fn delete(&self, id: ProjectId) -> RepoResult<()> {
        execute(
            &self.connection,
            "DELETE FROM projects WHERE id = ?1",
            params![id_text(id.as_uuid())],
        )
    }
}
