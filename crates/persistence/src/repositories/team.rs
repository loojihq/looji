use looji_domain::{
    ProjectId, RepoResult, TeamMember, TeamMemberId, TeamMemberRecord, TeamMemberRepository,
    Timestamp,
};
use rusqlite::{Row, params};

use super::{execute, get, id_text, query_all, query_one, read_id, read_timestamp, storage};
use crate::SqliteStore;

const MEMBER_COLUMNS: &str = "id, project_id, name, colour, created_at, updated_at";

fn read_member(row: &Row<'_>) -> RepoResult<TeamMemberRecord> {
    Ok(TeamMemberRecord {
        id: read_id(row, 0, TeamMemberId::from_uuid)?,
        project_id: read_id(row, 1, ProjectId::from_uuid)?,
        name: get(row, 2)?,
        colour: get(row, 3)?,
        created_at: read_timestamp(row, 4)?,
        updated_at: read_timestamp(row, 5)?,
    })
}

impl TeamMemberRepository for SqliteStore {
    /// Adds a member, or saves a rename. The project and colour are only set on insert, so a save
    /// can't move a member or change its colour.
    fn save(&self, member: &TeamMember) -> RepoResult<()> {
        execute(
            &self.connection,
            "INSERT INTO team_members (id, project_id, name, colour, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT (id) DO UPDATE SET
                 name = excluded.name,
                 updated_at = excluded.updated_at",
            params![
                id_text(member.id().as_uuid()),
                id_text(member.project_id().as_uuid()),
                member.name(),
                member.colour().index() as i64,
                member.created_at().unix_millis(),
                member.updated_at().unix_millis(),
            ],
        )
    }

    fn find(&self, id: TeamMemberId) -> RepoResult<Option<TeamMember>> {
        let sql = format!("SELECT {MEMBER_COLUMNS} FROM team_members WHERE id = ?1");
        let record = query_one(
            &self.connection,
            &sql,
            params![id_text(id.as_uuid())],
            read_member,
        )?;
        Ok(record.map(TeamMember::try_from).transpose()?)
    }

    /// Ordered by `rowid`, which is the order they were inserted in. `created_at` can't do this: two
    /// members added in the same millisecond would tie, and the suffixes would change between loads.
    fn list_for_project(&self, project: ProjectId) -> RepoResult<Vec<TeamMember>> {
        let sql = format!(
            "SELECT {MEMBER_COLUMNS} FROM team_members WHERE project_id = ?1 ORDER BY rowid"
        );
        query_all(
            &self.connection,
            &sql,
            params![id_text(project.as_uuid())],
            read_member,
        )?
        .into_iter()
        .map(|record| Ok(TeamMember::try_from(record)?))
        .collect()
    }

    fn assigned_task_count(&self, id: TeamMemberId) -> RepoResult<u64> {
        let count: i64 = self
            .connection
            .query_row(
                "SELECT COUNT(*) FROM tasks WHERE assignee_id = ?1",
                params![id_text(id.as_uuid())],
                |row| row.get(0),
            )
            .map_err(storage)?;
        Ok(u64::try_from(count).unwrap_or(0))
    }

    /// Unassigns the member's tasks, records each unassignment in their history, and deletes the
    /// member, all in one transaction. The tasks themselves are never deleted.
    fn remove(&self, id: TeamMemberId) -> RepoResult<u64> {
        let key = id_text(id.as_uuid());
        let now = Timestamp::now();
        let transaction = self.connection.unchecked_transaction().map_err(storage)?;

        let task_ids: Vec<String> = query_all(
            &transaction,
            "SELECT id FROM tasks WHERE assignee_id = ?1",
            params![key],
            |row| get(row, 0),
        )?;
        for task in &task_ids {
            transaction
                .execute(
                    "INSERT INTO task_events (id, task_id, kind, payload, created_at)
                     VALUES (?1, ?2, 'edited', '[\"assignee\"]', ?3)",
                    params![id_text(uuid::Uuid::new_v4()), task, now.unix_millis()],
                )
                .map_err(storage)?;
        }
        // The explicit clear doesn't depend on foreign keys being enabled, which the FK would need.
        transaction
            .execute(
                "UPDATE tasks SET assignee_id = NULL, updated_at = ?2 WHERE assignee_id = ?1",
                params![key, now.unix_millis()],
            )
            .map_err(storage)?;
        transaction
            .execute("DELETE FROM team_members WHERE id = ?1", params![key])
            .map_err(storage)?;
        transaction.commit().map_err(storage)?;
        Ok(u64::try_from(task_ids.len()).unwrap_or(0))
    }
}
