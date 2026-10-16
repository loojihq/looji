//! Planning drafts in SQLite. A draft is written whole, in one transaction, so a reopened draft is the last save.
//! Edits and refinements go through `apply_edit` and `apply_refinement`, which load the draft, apply the change, and
//! save it, so no change can be left unsaved.

use looji_domain::{
    DomainError, DraftState, Estimate, PlanDraft, PlanDraftError, PlanDraftId, PlanDraftParts,
    PlanEdit, PlanStory, PlanTask, Priority, ProjectId, RefineTarget, RefinedPart, RepoError,
    RepoResult, TeamMemberId, Timestamp,
};
use rusqlite::{Connection, Row, params};

use super::{
    execute, get, id_text, query_all, query_one, read_id, read_optional_id, read_timestamp, storage,
};
use crate::SqliteStore;

use super::task::write_task;
use looji_domain::{Position, Publication, Status, Task, after};

const DRAFT_COLUMNS: &str = "id, project_id, prompt, state, response, recovered, failure, created_at, updated_at, spec, spec_edited, published_at";

fn invalid(err: PlanDraftError) -> RepoError {
    RepoError::Invalid(DomainError::from(err))
}

/// Reads a draft's own row. Its stories and tasks are read separately, and filled in by `assemble`.
fn read_draft(row: &Row<'_>) -> RepoResult<PlanDraftParts> {
    let state: String = get(row, 3)?;
    let spec_edited: i64 = get(row, 10)?;
    let published_at: Option<i64> = get(row, 11)?;
    Ok(PlanDraftParts {
        id: read_id(row, 0, PlanDraftId::from_uuid)?,
        project_id: read_id(row, 1, ProjectId::from_uuid)?,
        prompt: get(row, 2)?,
        state: state.parse::<DraftState>().map_err(invalid)?,
        response: get(row, 4)?,
        recovered: get(row, 5)?,
        failure: get(row, 6)?,
        spec: get(row, 9)?,
        spec_edited: spec_edited != 0,
        published_at: published_at.map(Timestamp::from_unix_millis),
        stories: Vec::new(),
        tasks: Vec::new(),
        created_at: read_timestamp(row, 7)?,
        updated_at: read_timestamp(row, 8)?,
    })
}

/// Tags are stored as one comma-separated string. Tags can't contain a comma, so this is exact.
fn join_tags(tags: &[String]) -> String {
    tags.join(",")
}

fn split_tags(text: &str) -> Vec<String> {
    if text.is_empty() {
        Vec::new()
    } else {
        text.split(',').map(str::to_string).collect()
    }
}

/// Reads a draft's stories and tasks, in the order they were saved, and checks the whole draft.
fn assemble(connection: &Connection, mut parts: PlanDraftParts) -> RepoResult<PlanDraft> {
    let key = id_text(parts.id.as_uuid());
    parts.stories = query_all(
        connection,
        "SELECT title, body, edited FROM plan_draft_sections WHERE draft_id = ?1 ORDER BY position",
        params![key],
        |row| {
            let edited: i64 = get(row, 2)?;
            Ok(PlanStory {
                title: get(row, 0)?,
                body: get(row, 1)?,
                edited: edited != 0,
            })
        },
    )?;
    parts.tasks = query_all(
        connection,
        "SELECT title, section_position, priority, estimate_minutes, assignee_id, tags, edited
         FROM plan_draft_tasks WHERE draft_id = ?1 ORDER BY position",
        params![key],
        read_task,
    )?;
    PlanDraft::from_parts(parts).map_err(invalid)
}

fn read_task(row: &Row<'_>) -> RepoResult<PlanTask> {
    let section: Option<i64> = get(row, 1)?;
    let priority: String = get(row, 2)?;
    let minutes: Option<i64> = get(row, 3)?;
    let tags: String = get(row, 5)?;
    let edited: i64 = get(row, 6)?;
    Ok(PlanTask {
        title: get(row, 0)?,
        priority: priority.parse::<Priority>().map_err(RepoError::Invalid)?,
        estimate: minutes
            .map(Estimate::from_minutes)
            .transpose()
            .map_err(RepoError::Invalid)?,
        assignee: read_optional_id(row, 4, TeamMemberId::from_uuid)?,
        tags: split_tags(&tags),
        section: section.map(|position| position as usize),
        edited: edited != 0,
    })
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

impl SqliteStore {
    /// The team members of a project, for checking an assignee.
    fn team_of(&self, project: ProjectId) -> RepoResult<Vec<TeamMemberId>> {
        query_all(
            &self.connection,
            "SELECT id FROM team_members WHERE project_id = ?1",
            params![id_text(project.as_uuid())],
            |row| read_id(row, 0, TeamMemberId::from_uuid),
        )
    }

    /// Loads a draft, changes it with `change` against the project's current team, and saves it. A refused change
    /// is `Rejected`, and nothing is written. `None` means there is no such draft.
    fn change_draft(
        &self,
        id: PlanDraftId,
        change: impl FnOnce(&mut PlanDraft, &[TeamMemberId]) -> Result<(), PlanDraftError>,
    ) -> RepoResult<Option<PlanDraft>> {
        let Some(mut draft) = looji_domain::PlanDraftRepository::find(self, id)? else {
            return Ok(None);
        };
        let members = self.team_of(draft.project_id())?;
        change(&mut draft, &members).map_err(|err| RepoError::Rejected(err.into()))?;
        looji_domain::PlanDraftRepository::save(self, &draft)?;
        Ok(Some(draft))
    }
}

impl looji_domain::PlanDraftRepository for SqliteStore {
    /// Writes the whole draft. The row, its stories, and its tasks change in one transaction, so a failure leaves
    /// the last saved draft as it was. The prompt and creation time are only set on insert.
    fn save(&self, draft: &PlanDraft) -> RepoResult<()> {
        let key = id_text(draft.id().as_uuid());
        let transaction = self.connection.unchecked_transaction().map_err(storage)?;
        transaction
            .execute(
                "INSERT INTO plan_drafts (id, project_id, prompt, state, response, recovered, failure,
                                          created_at, updated_at, spec, spec_edited, published_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
                 ON CONFLICT (id) DO UPDATE SET
                     state = excluded.state,
                     response = excluded.response,
                     recovered = excluded.recovered,
                     failure = excluded.failure,
                     updated_at = excluded.updated_at,
                     spec = excluded.spec,
                     spec_edited = excluded.spec_edited,
                     published_at = excluded.published_at",
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
                    draft.spec(),
                    draft.spec_edited(),
                    draft.published_at().map(|at| at.unix_millis()),
                ],
            )
            .map_err(storage)?;

        // Tasks go first, since they refer to stories.
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
        for (position, story) in draft.stories().iter().enumerate() {
            transaction
                .execute(
                    "INSERT INTO plan_draft_sections (draft_id, position, title, body, edited)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![key, position as i64, story.title, story.body, story.edited],
                )
                .map_err(storage)?;
        }
        for (position, task) in draft.tasks().iter().enumerate() {
            transaction
                .execute(
                    "INSERT INTO plan_draft_tasks (draft_id, position, title, section_position, priority,
                                                   estimate_minutes, assignee_id, tags, edited)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    params![
                        key,
                        position as i64,
                        task.title,
                        task.section.map(|section| section as i64),
                        task.priority.as_str(),
                        task.estimate.map(|estimate| i64::from(estimate.minutes())),
                        task.assignee.map(|member| id_text(member.as_uuid())),
                        join_tags(&task.tags),
                        task.edited,
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

    /// Applies one edit. The team is read from storage here, so an assignee who was removed can't be assigned by a
    /// stale copy of the team in the UI.
    fn apply_edit(
        &self,
        id: PlanDraftId,
        edit: PlanEdit,
        at: Timestamp,
    ) -> RepoResult<Option<PlanDraft>> {
        self.change_draft(id, |draft, members| draft.apply(edit, members, at))
    }

    /// Replaces one part of a ready plan, and saves it. Every other part is saved as it was. A part a person edited
    /// by hand is refused unless `replace_edited` is set.
    fn apply_refinement(
        &self,
        id: PlanDraftId,
        target: RefineTarget,
        part: RefinedPart,
        replace_edited: bool,
        at: Timestamp,
    ) -> RepoResult<Option<PlanDraft>> {
        self.change_draft(id, |draft, members| {
            draft.refine(target, part, members, replace_edited, at)
        })
    }

    /// Publishes a ready plan's tasks into the project's Backlog column, in one transaction. The tasks go after the
    /// Backlog tasks already there, in the order the plan has them. A title the project already has is skipped. The
    /// draft is marked published in the same transaction, so a failure part way leaves no task behind and the draft
    /// unpublished. `None` means there is no such draft.
    fn publish(&self, id: PlanDraftId, at: Timestamp) -> RepoResult<Option<Publication>> {
        let Some(draft) = looji_domain::PlanDraftRepository::find(self, id)? else {
            return Ok(None);
        };
        let project = draft.project_id();
        let existing: Vec<String> = query_all(
            &self.connection,
            "SELECT title FROM tasks WHERE project_id = ?1",
            params![id_text(project.as_uuid())],
            |row| get(row, 0),
        )?;
        let plan = draft
            .publication_plan(&existing)
            .map_err(|err| RepoError::Rejected(err.into()))?;

        let transaction = self.connection.unchecked_transaction().map_err(storage)?;
        let mut last = last_backlog_position(&transaction, project)?;
        let mut published = Vec::new();
        for planned in &plan.create {
            let key = after(last.as_deref()).map_err(RepoError::Invalid)?;
            let position = Position::new(key.clone()).map_err(RepoError::Invalid)?;
            let mut task = Task::new(project, &planned.title, Status::Backlog, position)
                .map_err(RepoError::Invalid)?;
            task.set_priority(Some(planned.priority));
            task.set_estimate(planned.estimate);
            task.set_tags(planned.tags.clone());
            task.assign(planned.assignee);
            write_task(&transaction, &task)?;
            published.push(task.id());
            last = Some(key);
        }

        let marked = transaction
            .execute(
                "UPDATE plan_drafts SET published_at = ?1, updated_at = ?1
                 WHERE id = ?2 AND state = 'ready' AND published_at IS NULL",
                params![at.unix_millis(), id_text(id.as_uuid())],
            )
            .map_err(storage)?;
        if marked != 1 {
            // Dropping the transaction without committing rolls back the tasks written above.
            return Err(RepoError::Rejected(PlanDraftError::AlreadyPublished.into()));
        }
        transaction.commit().map_err(storage)?;
        Ok(Some(Publication {
            published,
            skipped: plan.skipped,
        }))
    }
}

/// The position of the last Backlog task in a project, or `None` when the Backlog is empty.
fn last_backlog_position(
    connection: &Connection,
    project: ProjectId,
) -> RepoResult<Option<String>> {
    Ok(query_all(
        connection,
        "SELECT position FROM tasks WHERE project_id = ?1 AND status = 'backlog'
         ORDER BY position COLLATE BINARY DESC, id DESC LIMIT 1",
        params![id_text(project.as_uuid())],
        |row| get(row, 0),
    )?
    .into_iter()
    .next())
}
