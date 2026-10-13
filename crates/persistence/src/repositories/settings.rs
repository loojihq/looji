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
        self.set_setting(ACTIVE_WORKSPACE_KEY, &id.as_uuid().to_string())
    }

    fn get_setting(&self, key: &str) -> RepoResult<Option<String>> {
        query_one(
            &self.connection,
            "SELECT value FROM settings WHERE key = ?1",
            params![key],
            |row| get::<String>(row, 0),
        )
    }

    fn set_setting(&self, key: &str, value: &str) -> RepoResult<()> {
        execute(
            &self.connection,
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )
    }

    fn remove_setting(&self, key: &str) -> RepoResult<()> {
        execute(
            &self.connection,
            "DELETE FROM settings WHERE key = ?1",
            params![key],
        )
    }
}
