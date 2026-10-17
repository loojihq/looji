//! Deleting a workspace and everything it owns, in one transaction.
//!
//! Foreign keys already cascade, but the deletes are written out here so the behaviour is
//! visible and testable. Every table that holds workspace data must appear in
//! `WORKSPACE_SCOPED`; a test checks the schema against this list.

use looji_domain::{DEFAULT_WORKSPACE_NAME, Workspace, WorkspaceId};
use rusqlite::{OptionalExtension, params};
use thiserror::Error;
use uuid::Uuid;

use crate::{SqliteStore, repositories::settings::ACTIVE_WORKSPACE_KEY};

/// Tables that hold data for a workspace, children first. Each statement takes the workspace id.
///
/// When a feature adds a table with a `workspace_id` or `project_id` column, add it here.
const WORKSPACE_SCOPED: &[(&str, &str)] = &[
    (
        "tasks",
        "DELETE FROM tasks WHERE project_id IN (SELECT id FROM projects WHERE workspace_id = ?1)",
    ),
    (
        "board_columns",
        "DELETE FROM board_columns WHERE project_id IN (SELECT id FROM projects WHERE workspace_id = ?1)",
    ),
    (
        "team_members",
        "DELETE FROM team_members WHERE project_id IN (SELECT id FROM projects WHERE workspace_id = ?1)",
    ),
    (
        "project_view_state",
        "DELETE FROM project_view_state WHERE project_id IN (SELECT id FROM projects WHERE workspace_id = ?1)",
    ),
    (
        "project_list_view",
        "DELETE FROM project_list_view WHERE project_id IN (SELECT id FROM projects WHERE workspace_id = ?1)",
    ),
    (
        "project_filters",
        "DELETE FROM project_filters WHERE project_id IN (SELECT id FROM projects WHERE workspace_id = ?1)",
    ),
    (
        "plan_drafts",
        "DELETE FROM plan_drafts WHERE project_id IN (SELECT id FROM projects WHERE workspace_id = ?1)",
    ),
    (
        "project_repository",
        "DELETE FROM project_repository WHERE project_id IN (SELECT id FROM projects WHERE workspace_id = ?1)",
    ),
    (
        "index_symbols",
        "DELETE FROM index_symbols WHERE project_id IN (SELECT id FROM projects WHERE workspace_id = ?1)",
    ),
    (
        "index_files",
        "DELETE FROM index_files WHERE project_id IN (SELECT id FROM projects WHERE workspace_id = ?1)",
    ),
    (
        "index_entries",
        "DELETE FROM index_entries WHERE project_id IN (SELECT id FROM projects WHERE workspace_id = ?1)",
    ),
    ("projects", "DELETE FROM projects WHERE workspace_id = ?1"),
];

/// Names of every table that holds workspace data, in deletion order.
pub fn workspace_scoped_tables() -> impl Iterator<Item = &'static str> {
    WORKSPACE_SCOPED.iter().map(|(table, _)| *table)
}

#[derive(Debug, Error)]
pub enum DeleteError {
    #[error("that workspace no longer exists")]
    NotFound,
    #[error("{0}")]
    Storage(String),
}

/// What a confirmed delete removed and where the selection went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeletedWorkspace {
    /// The workspace that is active after the delete.
    pub active: WorkspaceId,
    /// Icon files the caller should now remove from disk. Only returned after a successful commit.
    pub icon_files: Vec<String>,
}

/// How much a workspace holds, for the confirmation dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkspaceSummary {
    pub projects: u64,
    pub tasks: u64,
}

impl SqliteStore {
    /// Counts what a workspace holds. Read-only.
    pub fn workspace_summary(&self, id: WorkspaceId) -> Result<WorkspaceSummary, DeleteError> {
        let key = id.as_uuid().to_string();
        let storage = |err: rusqlite::Error| DeleteError::Storage(err.to_string());
        let exists: bool = self
            .connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM workspaces WHERE id = ?1)",
                params![key],
                |row| row.get(0),
            )
            .map_err(storage)?;
        if !exists {
            return Err(DeleteError::NotFound);
        }

        let projects: i64 = self
            .connection
            .query_row(
                "SELECT COUNT(*) FROM projects WHERE workspace_id = ?1",
                params![key],
                |row| row.get(0),
            )
            .map_err(storage)?;
        let tasks: i64 = self
            .connection
            .query_row(
                "SELECT COUNT(*) FROM tasks
                 WHERE project_id IN (SELECT id FROM projects WHERE workspace_id = ?1)",
                params![key],
                |row| row.get(0),
            )
            .map_err(storage)?;
        Ok(WorkspaceSummary {
            projects: u64::try_from(projects).unwrap_or(0),
            tasks: u64::try_from(tasks).unwrap_or(0),
        })
    }

    /// Deletes a workspace and everything it owns in one transaction.
    ///
    /// `next` is the workspace to select afterwards. If it no longer exists, the first remaining
    /// workspace is used. If no workspace remains, a default one is created in the same
    /// transaction, so a failure never leaves the app with zero workspaces.
    ///
    /// On error nothing is removed. Icon files are not touched here: the caller removes
    /// `DeletedWorkspace::icon_files` after this returns.
    pub fn delete_workspace(
        &self,
        id: WorkspaceId,
        next: Option<WorkspaceId>,
    ) -> Result<DeletedWorkspace, DeleteError> {
        let storage = |err: rusqlite::Error| DeleteError::Storage(err.to_string());
        let key = id.as_uuid().to_string();

        let transaction = crate::write_transaction(&self.connection).map_err(storage)?;

        let icon_file: Option<Option<String>> = transaction
            .query_row(
                "SELECT icon_file FROM workspaces WHERE id = ?1",
                params![key],
                |row| row.get(0),
            )
            .optional()
            .map_err(storage)?;
        let icon_file = icon_file.ok_or(DeleteError::NotFound)?;

        for (_, sql) in WORKSPACE_SCOPED {
            transaction.execute(sql, params![key]).map_err(storage)?;
        }
        transaction
            .execute("DELETE FROM workspaces WHERE id = ?1", params![key])
            .map_err(storage)?;

        let mut remaining = remaining_ids(&transaction).map_err(storage)?;
        if remaining.is_empty() {
            let default =
                Workspace::new(DEFAULT_WORKSPACE_NAME).expect("the default name is valid");
            transaction
                .execute(
                    "INSERT INTO workspaces (id, name, icon_file, created_at, updated_at)
                     VALUES (?1, ?2, NULL, ?3, ?4)",
                    params![
                        default.id().as_uuid().to_string(),
                        default.name(),
                        default.created_at().unix_millis(),
                        default.updated_at().unix_millis(),
                    ],
                )
                .map_err(storage)?;
            remaining.push(default.id());
        }

        let active = next
            .filter(|next| remaining.contains(next))
            .unwrap_or(remaining[0]);
        transaction
            .execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2)
                 ON CONFLICT (key) DO UPDATE SET value = excluded.value",
                params![ACTIVE_WORKSPACE_KEY, active.as_uuid().to_string()],
            )
            .map_err(storage)?;

        transaction.commit().map_err(storage)?;
        Ok(DeletedWorkspace {
            active,
            icon_files: icon_file.into_iter().collect(),
        })
    }
}

/// Ids of the remaining workspaces in sidebar order.
fn remaining_ids(connection: &rusqlite::Connection) -> rusqlite::Result<Vec<WorkspaceId>> {
    let mut statement = connection.prepare("SELECT id FROM workspaces ORDER BY created_at, id")?;
    let texts = statement
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<String>>>()?;
    Ok(texts
        .iter()
        .filter_map(|text| Uuid::parse_str(text).ok())
        .map(WorkspaceId::from_uuid)
        .collect())
}
