use looji_domain::{
    RepoResult, TeamMember, TeamMemberId, TeamMemberRecord, TeamMemberRepository, WorkspaceId,
};
use rusqlite::{Row, params};

use super::{execute, get, id_text, query_all, query_one, read_id, read_timestamp};
use crate::SqliteStore;

fn read_member(row: &Row<'_>) -> RepoResult<TeamMemberRecord> {
    Ok(TeamMemberRecord {
        id: read_id(row, 0, TeamMemberId::from_uuid)?,
        workspace_id: read_id(row, 1, WorkspaceId::from_uuid)?,
        name: get(row, 2)?,
        created_at: read_timestamp(row, 3)?,
        updated_at: read_timestamp(row, 4)?,
    })
}

impl TeamMemberRepository for SqliteStore {
    fn save(&self, member: &TeamMember) -> RepoResult<()> {
        execute(
            &self.connection,
            "INSERT INTO team_members (id, workspace_id, name, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (id) DO UPDATE SET
                 workspace_id = excluded.workspace_id,
                 name = excluded.name,
                 updated_at = excluded.updated_at",
            params![
                id_text(member.id().as_uuid()),
                id_text(member.workspace_id().as_uuid()),
                member.name(),
                member.created_at().unix_millis(),
                member.updated_at().unix_millis(),
            ],
        )
    }

    fn find(&self, id: TeamMemberId) -> RepoResult<Option<TeamMember>> {
        let record = query_one(
            &self.connection,
            "SELECT id, workspace_id, name, created_at, updated_at FROM team_members WHERE id = ?1",
            params![id_text(id.as_uuid())],
            read_member,
        )?;
        Ok(record.map(TeamMember::try_from).transpose()?)
    }

    fn list_for_workspace(&self, workspace: WorkspaceId) -> RepoResult<Vec<TeamMember>> {
        query_all(
            &self.connection,
            "SELECT id, workspace_id, name, created_at, updated_at FROM team_members
             WHERE workspace_id = ?1
             ORDER BY name, id",
            params![id_text(workspace.as_uuid())],
            read_member,
        )?
        .into_iter()
        .map(|record| Ok(TeamMember::try_from(record)?))
        .collect()
    }

    fn delete(&self, id: TeamMemberId) -> RepoResult<()> {
        execute(
            &self.connection,
            "DELETE FROM team_members WHERE id = ?1",
            params![id_text(id.as_uuid())],
        )
    }
}
