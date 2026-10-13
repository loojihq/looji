use looji_domain::{DomainError, RepoResult, SettingsRepository, WorkspaceId};
use rusqlite::params;
use uuid::Uuid;

use super::{execute, get, query_one};
use crate::SqliteStore;

pub(crate) const ACTIVE_WORKSPACE_KEY: &str = "active_workspace_id";

impl SettingsRepository for SqliteStore {
    fn active_workspace(&self) -> RepoResult<Option<WorkspaceId>> {
        let stored = query_one(
            &self.connection,
            "SELECT value FROM settings WHERE key = ?1",
            params![ACTIVE_WORKSPACE_KEY],
            |row| get::<String>(row, 0),
        )?;
        let id = stored
            .map(|text| {
                Uuid::parse_str(&text)
                    .map(WorkspaceId::from_uuid)
                    .map_err(|_| DomainError::InvalidId(text))
            })
            .transpose()?;
        Ok(id)
    }

    fn set_active_workspace(&self, id: WorkspaceId) -> RepoResult<()> {
        execute(
            &self.connection,
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            params![ACTIVE_WORKSPACE_KEY, id.as_uuid().to_string()],
        )
    }
}
