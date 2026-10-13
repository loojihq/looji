use looji_domain::{
    RepoError, RepoResult, Workspace, WorkspaceId, WorkspaceRecord, WorkspaceRepository,
};
use rusqlite::{Row, params};

use super::{execute, get, id_text, query_all, query_one, read_id, read_timestamp, storage};
use crate::SqliteStore;

const NAME_INDEX: &str = "workspaces_name_unique";

fn read_workspace(row: &Row<'_>) -> RepoResult<WorkspaceRecord> {
    Ok(WorkspaceRecord {
        id: read_id(row, 0, WorkspaceId::from_uuid)?,
        name: get(row, 1)?,
        icon_file: get(row, 2)?,
        created_at: read_timestamp(row, 3)?,
        updated_at: read_timestamp(row, 4)?,
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

impl WorkspaceRepository for SqliteStore {
    /// Inserts or updates the name and timestamps. The icon is written only on insert:
    /// later icon changes go through `SqliteStore::replace_workspace_icon`, so a rename
    /// can't overwrite an icon that changed elsewhere.
    fn save(&self, workspace: &Workspace) -> RepoResult<()> {
        self.connection
            .execute(
                "INSERT INTO workspaces (id, name, icon_file, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT (id) DO UPDATE SET
                     name = excluded.name,
                     updated_at = excluded.updated_at",
                params![
                    id_text(workspace.id().as_uuid()),
                    workspace.name(),
                    workspace.icon_file(),
                    workspace.created_at().unix_millis(),
                    workspace.updated_at().unix_millis(),
                ],
            )
            .map_err(save_error)?;
        Ok(())
    }

    fn find(&self, id: WorkspaceId) -> RepoResult<Option<Workspace>> {
        let record = query_one(
            &self.connection,
            "SELECT id, name, icon_file, created_at, updated_at FROM workspaces WHERE id = ?1",
            params![id_text(id.as_uuid())],
            read_workspace,
        )?;
        Ok(record.map(Workspace::try_from).transpose()?)
    }

    fn list(&self) -> RepoResult<Vec<Workspace>> {
        query_all(
            &self.connection,
            "SELECT id, name, icon_file, created_at, updated_at FROM workspaces
             ORDER BY created_at, id",
            [],
            read_workspace,
        )?
        .into_iter()
        .map(|record| Ok(Workspace::try_from(record)?))
        .collect()
    }

    fn delete(&self, id: WorkspaceId) -> RepoResult<()> {
        execute(
            &self.connection,
            "DELETE FROM workspaces WHERE id = ?1",
            params![id_text(id.as_uuid())],
        )
    }
}
