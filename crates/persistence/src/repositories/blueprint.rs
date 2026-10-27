//! A project's blueprint. The current documents are in `blueprint_docs`. Writing a new blueprint moves the ones it
//! replaces into `blueprint_history` and numbers the new set one higher.

use looji_domain::{
    BlueprintDoc, BlueprintRepository, ProjectId, RepoError, RepoResult, Timestamp,
};
use rusqlite::params;

use super::{get, id_text, query_all, read_timestamp, storage};
use crate::SqliteStore;

impl BlueprintRepository for SqliteStore {
    fn blueprint(&self, project: ProjectId) -> RepoResult<Vec<BlueprintDoc>> {
        query_all(
            &self.connection,
            "SELECT kind, title, body, diagrams, version, updated_at FROM blueprint_docs
             WHERE project_id = ?1 ORDER BY position",
            params![id_text(project.as_uuid())],
            |row| {
                let version: i64 = get(row, 4)?;
                Ok(BlueprintDoc {
                    kind: get(row, 0)?,
                    title: get(row, 1)?,
                    body: get(row, 2)?,
                    diagrams: get(row, 3)?,
                    version: u32::try_from(version)
                        .map_err(|_| RepoError::Storage("bad blueprint version".to_string()))?,
                    updated_at: read_timestamp(row, 5)?,
                })
            },
        )
    }

    fn blueprint_versions(&self, project: ProjectId) -> RepoResult<Vec<u32>> {
        let key = id_text(project.as_uuid());
        let mut versions: Vec<u32> = query_all(
            &self.connection,
            "SELECT version FROM blueprint_docs WHERE project_id = ?1
             UNION SELECT version FROM blueprint_history WHERE project_id = ?1",
            params![key],
            |row| {
                let version: i64 = get(row, 0)?;
                u32::try_from(version).map_err(|_| RepoError::Storage("bad version".to_string()))
            },
        )?;
        versions.sort_unstable_by(|a, b| b.cmp(a));
        versions.dedup();
        Ok(versions)
    }

    fn blueprint_version(&self, project: ProjectId, version: u32) -> RepoResult<Vec<BlueprintDoc>> {
        let read: fn(&rusqlite::Row<'_>) -> RepoResult<BlueprintDoc> = |row| {
            let version: i64 = get(row, 4)?;
            Ok(BlueprintDoc {
                kind: get(row, 0)?,
                title: get(row, 1)?,
                body: get(row, 2)?,
                diagrams: get(row, 3)?,
                version: u32::try_from(version)
                    .map_err(|_| RepoError::Storage("bad blueprint version".to_string()))?,
                updated_at: read_timestamp(row, 5)?,
            })
        };
        let key = id_text(project.as_uuid());
        let current = query_all(
            &self.connection,
            "SELECT kind, title, body, diagrams, version, updated_at FROM blueprint_docs
             WHERE project_id = ?1 AND version = ?2 ORDER BY position",
            params![key, version],
            read,
        )?;
        if !current.is_empty() {
            return Ok(current);
        }
        query_all(
            &self.connection,
            "SELECT kind, title, body, diagrams, version, updated_at FROM blueprint_history
             WHERE project_id = ?1 AND version = ?2 ORDER BY position",
            params![key, version],
            read,
        )
    }

    fn replace_blueprint(
        &self,
        project: ProjectId,
        docs: &[BlueprintDoc],
        at: Timestamp,
    ) -> RepoResult<u32> {
        let key = id_text(project.as_uuid());
        let transaction = crate::write_transaction(&self.connection).map_err(storage)?;
        let previous: i64 = transaction
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM blueprint_docs WHERE project_id = ?1",
                params![key],
                |row| row.get(0),
            )
            .map_err(storage)?;
        transaction
            .execute(
                "INSERT INTO blueprint_history
                     (project_id, version, position, kind, title, body, diagrams, updated_at)
                 SELECT project_id, version, position, kind, title, body, diagrams, updated_at
                 FROM blueprint_docs WHERE project_id = ?1",
                params![key],
            )
            .map_err(storage)?;
        transaction
            .execute(
                "DELETE FROM blueprint_docs WHERE project_id = ?1",
                params![key],
            )
            .map_err(storage)?;
        let version = previous + 1;
        for (position, doc) in docs.iter().enumerate() {
            transaction
                .execute(
                    "INSERT INTO blueprint_docs
                         (id, project_id, position, kind, title, body, diagrams, version, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    params![
                        id_text(uuid::Uuid::new_v4()),
                        key,
                        position as i64,
                        doc.kind,
                        doc.title,
                        doc.body,
                        doc.diagrams,
                        version,
                        at.unix_millis(),
                    ],
                )
                .map_err(storage)?;
        }
        transaction.commit().map_err(storage)?;
        u32::try_from(version).map_err(|_| RepoError::Storage("too many versions".to_string()))
    }
}
