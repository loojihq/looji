//! Saving a project's working hours. The draft is checked here, in the save path, so no caller can write
//! hours that break a rule. The write runs in one transaction, and a refused draft writes nothing.
//!
//! Existing due dates aren't recalculated. A date was set when its task was scheduled, and new hours only
//! apply to tasks that move into progress from then on.

use looji_domain::{DraftIssue, ProjectId, ScheduleDraft, WorkingSchedule};
use rusqlite::params;
use thiserror::Error;

use crate::SqliteStore;

#[derive(Debug, Error)]
pub enum ScheduleSaveError {
    /// The draft breaks at least one rule. Each rule is listed, in the order the screen reads them.
    #[error("the working hours are not valid")]
    Invalid(Vec<DraftIssue>),
    #[error("that project no longer exists")]
    ProjectNotFound,
    #[error("the working hours could not be saved: {0}")]
    Storage(String),
}

impl SqliteStore {
    /// Checks the draft, then writes it as the project's working hours in one transaction. Returns the
    /// schedule that was stored.
    pub fn save_working_schedule(
        &self,
        project: ProjectId,
        draft: &ScheduleDraft,
    ) -> Result<WorkingSchedule, ScheduleSaveError> {
        let schedule = draft.to_schedule().map_err(ScheduleSaveError::Invalid)?;
        let storage = |err: rusqlite::Error| ScheduleSaveError::Storage(err.to_string());
        let key = project.as_uuid().to_string();

        let transaction = self.connection.unchecked_transaction().map_err(storage)?;
        let updated = transaction
            .execute(
                "UPDATE projects
                 SET working_start_min = ?1, working_end_min = ?2, workdays_mask = ?3
                 WHERE id = ?4",
                params![
                    i64::from(schedule.start()),
                    i64::from(schedule.end()),
                    i64::from(schedule.workdays().mask()),
                    key,
                ],
            )
            .map_err(storage)?;
        if updated == 0 {
            return Err(ScheduleSaveError::ProjectNotFound);
        }
        transaction.commit().map_err(storage)?;
        Ok(schedule)
    }
}
