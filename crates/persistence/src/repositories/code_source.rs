//! The repository folder each project is connected to. The stored path is what the user chose, trimmed. Whether
//! it still exists is decided when the project loads, so nothing here clears a path on its own.

use looji_domain::{CodeSourceRepository, ProjectId, RepoResult, Timestamp};
use rusqlite::params;

use super::{execute, get, id_text, query_one};
use crate::SqliteStore;

impl CodeSourceRepository for SqliteStore {
    fn load_path(&self, project: ProjectId) -> RepoResult<Option<String>> {
        query_one(
            &self.connection,
            "SELECT path FROM project_repository WHERE project_id = ?1",
            params![id_text(project.as_uuid())],
            |row| get(row, 0),
        )
    }

    fn save_path(&self, project: ProjectId, path: &str) -> RepoResult<()> {
        execute(
            &self.connection,
            "INSERT INTO project_repository (project_id, path, connected_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT (project_id) DO UPDATE SET
                 path = excluded.path,
                 connected_at = excluded.connected_at",
            params![
                id_text(project.as_uuid()),
                path,
                Timestamp::now().unix_millis(),
            ],
        )
    }

    fn clear_path(&self, project: ProjectId) -> RepoResult<()> {
        execute(
            &self.connection,
            "DELETE FROM project_repository WHERE project_id = ?1",
            params![id_text(project.as_uuid())],
        )
    }
}
