//! Planning drafts in SQLite. A draft is written whole, in one transaction, so a reopened draft is the last save.

use looji_domain::{
    DomainError, DraftState, PlanDraft, PlanDraftError, PlanDraftId, PlanDraftParts,
    PlanDraftRepository, PlanSection, PlanTask, ProjectId, RepoError, RepoResult, Timestamp,
};
use rusqlite::{Connection, Row, params};

use super::{execute, get, id_text, query_all, query_one, read_id, read_timestamp, storage};
use crate::SqliteStore;

const DRAFT_COLUMNS: &str =
    "id, project_id, prompt, state, response, recovered, failure, created_at, updated_at";

fn invalid(err: PlanDraftError) -> RepoError {
    RepoError::Invalid(DomainError::from(err))
}

/// Reads a draft's own row. Its sections and tasks are read separately, and filled in by `assemble`.
fn read_draft(row: &Row<'_>) -> RepoResult<PlanDraftParts> {
    let state: String = get(row, 3)?;
    Ok(PlanDraftParts {
        id: read_id(row, 0, PlanDraftId::from_uuid)?,
        project_id: read_id(row, 1, ProjectId::from_uuid)?,
        prompt: get(row, 2)?,
        state: state.parse::<DraftState>().map_err(invalid)?,
        response: get(row, 4)?,
        recovered: get(row, 5)?,
        failure: get(row, 6)?,
        sections: Vec::new(),
        tasks: Vec::new(),
        created_at: read_timestamp(row, 7)?,
        updated_at: read_timestamp(row, 8)?,
    })
}

/// Reads a draft's sections and tasks, in the order they were saved, and checks the whole draft.
fn assemble(connection: &Connection, mut parts: PlanDraftParts) -> RepoResult<PlanDraft> {
    let key = id_text(parts.id.as_uuid());
    parts.sections = query_all(
        connection,
        "SELECT title, body FROM plan_draft_sections WHERE draft_id = ?1 ORDER BY position",
        params![key],
        |row| {
            Ok(PlanSection {
                title: get(row, 0)?,
                body: get(row, 1)?,
            })
        },
    )?;
    parts.tasks = query_all(
        connection,
        "SELECT title, section_position FROM plan_draft_tasks WHERE draft_id = ?1 ORDER BY position",
        params![key],
        |row| {
            let section: Option<i64> = get(row, 1)?;
            Ok(PlanTask {
                title: get(row, 0)?,
                section: section.map(|position| position as usize),
            })
        },
    )?;
    PlanDraft::from_parts(parts).map_err(invalid)
}

/// Marks every generating draft as interrupted. A generation runs in the process that started it, so one still
/// generating when a store opens was cut off by the app closing. Returns how many drafts changed.
pub(crate) fn interrupt_generating(
    connection: &Connection,
    at: Timestamp,
) -> rusqlite::Result<usize> {
    connection.execute(
        "UPDATE plan_drafts SET state = 'interrupted', updated_at = ?1 WHERE state = 'generating'",
        params![at.unix_millis()],
    )
}

impl PlanDraftRepository for SqliteStore {
    /// Writes the whole draft. The row, its sections, and its tasks change in one transaction, so a failure
    /// leaves the last saved draft as it was. The prompt and creation time are only set on insert.
    fn save(&self, draft: &PlanDraft) -> RepoResult<()> {
        let key = id_text(draft.id().as_uuid());
        let transaction = self.connection.unchecked_transaction().map_err(storage)?;
        transaction
            .execute(
                "INSERT INTO plan_drafts (id, project_id, prompt, state, response, recovered, failure,
                                          created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                 ON CONFLICT (id) DO UPDATE SET
                     state = excluded.state,
                     response = excluded.response,
                     recovered = excluded.recovered,
                     failure = excluded.failure,
                     updated_at = excluded.updated_at",
                params![
                    key,
                    id_text(draft.project_id().as_uuid()),
                    draft.prompt(),
                    draft.state().as_str(),
                    draft.response(),
                    draft.recovered(),
                    draft.failure(),
                    draft.created_at().unix_millis(),
                    draft.updated_at().unix_millis(),
                ],
            )
            .map_err(storage)?;

        // Tasks go first, since they refer to sections.
        transaction
            .execute(
                "DELETE FROM plan_draft_tasks WHERE draft_id = ?1",
                params![key],
            )
            .map_err(storage)?;
        transaction
            .execute(
                "DELETE FROM plan_draft_sections WHERE draft_id = ?1",
                params![key],
            )
            .map_err(storage)?;
        for (position, section) in draft.sections().iter().enumerate() {
            transaction
                .execute(
                    "INSERT INTO plan_draft_sections (draft_id, position, title, body)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![key, position as i64, section.title, section.body],
                )
                .map_err(storage)?;
        }
        for (position, task) in draft.tasks().iter().enumerate() {
            transaction
                .execute(
                    "INSERT INTO plan_draft_tasks (draft_id, position, title, section_position)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![
                        key,
                        position as i64,
                        task.title,
                        task.section.map(|section| section as i64),
                    ],
                )
                .map_err(storage)?;
        }
        transaction.commit().map_err(storage)
    }

    fn find(&self, id: PlanDraftId) -> RepoResult<Option<PlanDraft>> {
        let sql = format!("SELECT {DRAFT_COLUMNS} FROM plan_drafts WHERE id = ?1");
        let parts = query_one(
            &self.connection,
            &sql,
            params![id_text(id.as_uuid())],
            read_draft,
        )?;
        parts
            .map(|parts| assemble(&self.connection, parts))
            .transpose()
    }

    fn list_for_project(&self, project: ProjectId) -> RepoResult<Vec<PlanDraft>> {
        let sql = format!(
            "SELECT {DRAFT_COLUMNS} FROM plan_drafts WHERE project_id = ?1
             ORDER BY created_at DESC, rowid DESC"
        );
        query_all(
            &self.connection,
            &sql,
            params![id_text(project.as_uuid())],
            read_draft,
        )?
        .into_iter()
        .map(|parts| assemble(&self.connection, parts))
        .collect()
    }

    fn delete(&self, id: PlanDraftId) -> RepoResult<()> {
        execute(
            &self.connection,
            "DELETE FROM plan_drafts WHERE id = ?1",
            params![id_text(id.as_uuid())],
        )
    }
}
