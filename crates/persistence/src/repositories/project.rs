use looji_domain::{Project, ProjectId, ProjectRecord, ProjectRepository, RepoResult, WorkspaceId};
use rusqlite::{Row, params};

use super::{execute, get, id_text, query_all, query_one, read_id, read_timestamp};
use crate::SqliteStore;

fn read_project(row: &Row<'_>) -> RepoResult<ProjectRecord> {
    Ok(ProjectRecord {
        id: read_id(row, 0, ProjectId::from_uuid)?,
        workspace_id: read_id(row, 1, WorkspaceId::from_uuid)?,
        name: get(row, 2)?,
        created_at: read_timestamp(row, 3)?,
        updated_at: read_timestamp(row, 4)?,
    })
}

impl ProjectRepository for SqliteStore {
    fn save(&self, project: &Project) -> RepoResult<()> {
        execute(
            &self.connection,
            "INSERT INTO projects (id, workspace_id, name, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (id) DO UPDATE SET
                 workspace_id = excluded.workspace_id,
                 name = excluded.name,
                 updated_at = excluded.updated_at",
            params![
                id_text(project.id().as_uuid()),
                id_text(project.workspace_id().as_uuid()),
                project.name(),
                project.created_at().unix_millis(),
                project.updated_at().unix_millis(),
            ],
        )
    }

    fn find(&self, id: ProjectId) -> RepoResult<Option<Project>> {
        let record = query_one(
            &self.connection,
            "SELECT id, workspace_id, name, created_at, updated_at FROM projects WHERE id = ?1",
            params![id_text(id.as_uuid())],
            read_project,
        )?;
        Ok(record.map(Project::try_from).transpose()?)
    }

    fn list_for_workspace(&self, workspace: WorkspaceId) -> RepoResult<Vec<Project>> {
        query_all(
            &self.connection,
            "SELECT id, workspace_id, name, created_at, updated_at FROM projects
             WHERE workspace_id = ?1
             ORDER BY created_at, id",
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
