//! Planning drafts: what a planning request produced, kept so it can be reopened, resumed, or discarded after
//! the app closes.
//!
//! The states:
//! - `Generating`: a request is in flight. Text is arriving.
//! - `Interrupted`: the app closed during generation. A generation can't outlive the process, so a draft still
//!   generating when the store next opens is interrupted.
//! - `Failed`: the request ended with an error, and the reason is kept.
//! - `Ready`: the response was read and the plan is here.
//!
//! Partial text is never discarded. An interrupted or failed draft keeps the text it received, since a partial
//! response can still hold a plan worth recovering. Resuming starts a new generation and moves the old text to
//! `recovered`, so a second failure doesn't lose it. A completed draft has nothing to recover, so it drops it.
//! Discarding deletes the draft; that's the repository's job, not a state here.
//!
//! The text is kept as it was received. This module doesn't parse it. The strict JSON parser turns the text into
//! sections and tasks, and `complete` is where those go in.

use std::{fmt, str::FromStr};

use thiserror::Error;

use crate::{
    error::DomainError,
    ids::{PlanDraftId, ProjectId},
    time::Timestamp,
};

/// Why a draft's state can't change the way it was asked to.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PlanDraftError {
    #[error("a planning request needs a prompt")]
    EmptyPrompt,
    #[error("a section needs a title")]
    EmptySectionTitle,
    #[error("a task needs a title")]
    EmptyTaskTitle,
    #[error("a task refers to section {section}, which doesn't exist")]
    UnknownSection { section: usize },
    #[error("a {state} draft can't {action}")]
    IllegalTransition {
        state: DraftState,
        action: &'static str,
    },
    #[error("'{0}' is not a planning draft state")]
    UnknownState(String),
}

/// Where a draft is in its life. The stored form is the lowercase name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DraftState {
    Generating,
    Interrupted,
    Ready,
    Failed,
}

impl DraftState {
    pub const ALL: [DraftState; 4] = [
        DraftState::Generating,
        DraftState::Interrupted,
        DraftState::Ready,
        DraftState::Failed,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            DraftState::Generating => "generating",
            DraftState::Interrupted => "interrupted",
            DraftState::Ready => "ready",
            DraftState::Failed => "failed",
        }
    }
}

impl fmt::Display for DraftState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for DraftState {
    type Err = PlanDraftError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        DraftState::ALL
            .into_iter()
            .find(|state| state.as_str() == text)
            .ok_or_else(|| PlanDraftError::UnknownState(text.to_string()))
    }
}

/// One part of the plan. The body is whatever the planning response said about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanSection {
    pub title: String,
    pub body: String,
}

/// A task the plan proposes. `section` is the index of the section it belongs to, if any.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanTask {
    pub title: String,
    pub section: Option<usize>,
}

/// Everything a stored draft holds. Used to rebuild a draft from storage without running the transitions again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanDraftParts {
    pub id: PlanDraftId,
    pub project_id: ProjectId,
    pub prompt: String,
    pub state: DraftState,
    pub response: String,
    pub recovered: Option<String>,
    pub failure: Option<String>,
    pub sections: Vec<PlanSection>,
    pub tasks: Vec<PlanTask>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanDraft {
    parts: PlanDraftParts,
}

/// Trims and checks a prompt. Blank prompts are refused.
fn clean_prompt(prompt: &str) -> Result<String, PlanDraftError> {
    let trimmed = prompt.trim();
    if trimmed.is_empty() {
        return Err(PlanDraftError::EmptyPrompt);
    }
    Ok(trimmed.to_string())
}

/// Every title is trimmed and must not be blank. Each task's section must be one of the sections given.
fn check_content(sections: &[PlanSection], tasks: &[PlanTask]) -> Result<(), PlanDraftError> {
    if sections
        .iter()
        .any(|section| section.title.trim().is_empty())
    {
        return Err(PlanDraftError::EmptySectionTitle);
    }
    for task in tasks {
        if task.title.trim().is_empty() {
            return Err(PlanDraftError::EmptyTaskTitle);
        }
        if let Some(section) = task.section
            && section >= sections.len()
        {
            return Err(PlanDraftError::UnknownSection { section });
        }
    }
    Ok(())
}

impl PlanDraft {
    /// A new draft for a prompt. It starts generating, and nothing has been received yet.
    pub fn start(project: ProjectId, prompt: &str, at: Timestamp) -> Result<Self, PlanDraftError> {
        Self::start_as(PlanDraftId::new(), project, prompt, at)
    }

    /// A new draft under an id chosen by the caller. The wizard names the draft before it exists, so it can record
    /// which draft is running.
    pub fn start_as(
        id: PlanDraftId,
        project: ProjectId,
        prompt: &str,
        at: Timestamp,
    ) -> Result<Self, PlanDraftError> {
        Ok(Self {
            parts: PlanDraftParts {
                id,
                project_id: project,
                prompt: clean_prompt(prompt)?,
                state: DraftState::Generating,
                response: String::new(),
                recovered: None,
                failure: None,
                sections: Vec::new(),
                tasks: Vec::new(),
                created_at: at,
                updated_at: at,
            },
        })
    }

    /// Rebuilds a draft from storage. Checks what the transitions check, so a stored draft that breaks a rule
    /// is refused rather than loaded.
    pub fn from_parts(parts: PlanDraftParts) -> Result<Self, PlanDraftError> {
        let prompt = clean_prompt(&parts.prompt)?;
        check_content(&parts.sections, &parts.tasks)?;
        Ok(Self {
            parts: PlanDraftParts { prompt, ..parts },
        })
    }

    pub fn id(&self) -> PlanDraftId {
        self.parts.id
    }

    pub fn project_id(&self) -> ProjectId {
        self.parts.project_id
    }

    pub fn prompt(&self) -> &str {
        &self.parts.prompt
    }

    pub fn state(&self) -> DraftState {
        self.parts.state
    }

    /// The text received so far, as it arrived.
    pub fn response(&self) -> &str {
        &self.parts.response
    }

    /// The text from an earlier attempt, kept after a resume until the plan completes.
    pub fn recovered(&self) -> Option<&str> {
        self.parts.recovered.as_deref()
    }

    /// Why the last attempt failed, when it failed.
    pub fn failure(&self) -> Option<&str> {
        self.parts.failure.as_deref()
    }

    pub fn sections(&self) -> &[PlanSection] {
        &self.parts.sections
    }

    pub fn tasks(&self) -> &[PlanTask] {
        &self.parts.tasks
    }

    pub fn created_at(&self) -> Timestamp {
        self.parts.created_at
    }

    pub fn updated_at(&self) -> Timestamp {
        self.parts.updated_at
    }

    /// Adds text that arrived from the response. Only a generating draft takes text.
    pub fn append_response(&mut self, text: &str, at: Timestamp) -> Result<(), PlanDraftError> {
        self.require(DraftState::Generating, "receive text")?;
        self.parts.response.push_str(text);
        self.parts.updated_at = at;
        Ok(())
    }

    /// The response was read and the plan is here. The text is kept as the response, and any recovered text from
    /// an earlier attempt is dropped, since the plan now stands on its own.
    pub fn complete(
        &mut self,
        sections: Vec<PlanSection>,
        tasks: Vec<PlanTask>,
        at: Timestamp,
    ) -> Result<(), PlanDraftError> {
        self.require(DraftState::Generating, "complete")?;
        check_content(&sections, &tasks)?;
        self.parts.sections = sections;
        self.parts.tasks = tasks;
        self.parts.state = DraftState::Ready;
        self.parts.recovered = None;
        self.parts.failure = None;
        self.parts.updated_at = at;
        Ok(())
    }

    /// The request ended with an error. The text received so far is kept.
    pub fn fail(&mut self, reason: &str, at: Timestamp) -> Result<(), PlanDraftError> {
        self.require(DraftState::Generating, "fail")?;
        self.parts.state = DraftState::Failed;
        self.parts.failure = Some(reason.to_string());
        self.parts.updated_at = at;
        Ok(())
    }

    /// The app closed while the draft was generating. The text received so far is kept.
    pub fn interrupt(&mut self, at: Timestamp) -> Result<(), PlanDraftError> {
        self.require(DraftState::Generating, "be interrupted")?;
        self.parts.state = DraftState::Interrupted;
        self.parts.updated_at = at;
        Ok(())
    }

    /// Starts a new generation for an interrupted or failed draft. The text from the last attempt moves to
    /// `recovered`, and a draft with no text keeps whatever it had recovered before.
    pub fn resume(&mut self, at: Timestamp) -> Result<(), PlanDraftError> {
        if !matches!(
            self.parts.state,
            DraftState::Interrupted | DraftState::Failed
        ) {
            return Err(PlanDraftError::IllegalTransition {
                state: self.parts.state,
                action: "resume",
            });
        }
        if !self.parts.response.is_empty() {
            self.parts.recovered = Some(std::mem::take(&mut self.parts.response));
        }
        self.parts.state = DraftState::Generating;
        self.parts.failure = None;
        self.parts.updated_at = at;
        Ok(())
    }

    fn require(&self, state: DraftState, action: &'static str) -> Result<(), PlanDraftError> {
        if self.parts.state == state {
            Ok(())
        } else {
            Err(PlanDraftError::IllegalTransition {
                state: self.parts.state,
                action,
            })
        }
    }
}

impl From<PlanDraftError> for DomainError {
    fn from(err: PlanDraftError) -> Self {
        DomainError::Draft(err)
    }
}

#[cfg(test)]
mod tests {
    // Not `use super::*`: gpui's `test` macro would shadow `#[test]`.
    use crate::{
        ids::ProjectId,
        plan_draft::{DraftState, PlanDraft, PlanDraftError, PlanSection, PlanTask},
        time::Timestamp,
    };

    fn at(millis: i64) -> Timestamp {
        Timestamp::from_unix_millis(millis)
    }

    fn generating() -> PlanDraft {
        PlanDraft::start(ProjectId::new(), "  Plan the launch  ", at(1)).unwrap()
    }

    fn section(title: &str) -> PlanSection {
        PlanSection {
            title: title.to_string(),
            body: String::new(),
        }
    }

    #[test]
    fn a_new_draft_is_generating_and_its_prompt_is_trimmed() {
        let draft = generating();
        assert_eq!(draft.state(), DraftState::Generating);
        assert_eq!(draft.prompt(), "Plan the launch");
        assert_eq!(draft.response(), "");
    }

    #[test]
    fn a_blank_prompt_is_refused() {
        assert_eq!(
            PlanDraft::start(ProjectId::new(), "  \n ", at(1)),
            Err(PlanDraftError::EmptyPrompt)
        );
    }

    #[test]
    fn text_accumulates_while_generating() {
        let mut draft = generating();
        draft.append_response("{\"sections\":[", at(2)).unwrap();
        draft.append_response("]}", at(3)).unwrap();
        assert_eq!(draft.response(), "{\"sections\":[]}");
        assert_eq!(draft.updated_at(), at(3));
    }

    #[test]
    fn completing_stores_the_plan_and_moves_to_ready() {
        let mut draft = generating();
        draft
            .complete(
                vec![section("Scope")],
                vec![PlanTask {
                    title: "Write the brief".to_string(),
                    section: Some(0),
                }],
                at(4),
            )
            .unwrap();
        assert_eq!(draft.state(), DraftState::Ready);
        assert_eq!(draft.sections().len(), 1);
        assert_eq!(draft.tasks()[0].section, Some(0));
    }

    #[test]
    fn a_task_in_a_section_that_doesnt_exist_is_refused() {
        let mut draft = generating();
        let result = draft.complete(
            vec![section("Scope")],
            vec![PlanTask {
                title: "Orphan".to_string(),
                section: Some(3),
            }],
            at(4),
        );
        assert_eq!(result, Err(PlanDraftError::UnknownSection { section: 3 }));
        assert_eq!(draft.state(), DraftState::Generating, "nothing changed");
    }

    #[test]
    fn a_blank_title_is_refused_for_a_section_and_for_a_task() {
        let mut draft = generating();
        assert_eq!(
            draft.complete(vec![section("  ")], Vec::new(), at(4)),
            Err(PlanDraftError::EmptySectionTitle)
        );
        assert_eq!(
            draft.complete(
                Vec::new(),
                vec![PlanTask {
                    title: String::new(),
                    section: None
                }],
                at(4)
            ),
            Err(PlanDraftError::EmptyTaskTitle)
        );
    }

    #[test]
    fn a_failure_keeps_the_text_received_and_its_reason() {
        let mut draft = generating();
        draft
            .append_response("{\"sections\":[{\"title\":\"Sc", at(2))
            .unwrap();
        draft.fail("the connection dropped", at(3)).unwrap();
        assert_eq!(draft.state(), DraftState::Failed);
        assert_eq!(draft.response(), "{\"sections\":[{\"title\":\"Sc");
        assert_eq!(draft.failure(), Some("the connection dropped"));
    }

    #[test]
    fn resuming_moves_the_old_text_to_recovered_and_starts_again() {
        let mut draft = generating();
        draft.append_response("partial", at(2)).unwrap();
        draft.interrupt(at(3)).unwrap();

        draft.resume(at(4)).unwrap();

        assert_eq!(draft.state(), DraftState::Generating);
        assert_eq!(draft.response(), "");
        assert_eq!(draft.recovered(), Some("partial"));
    }

    #[test]
    fn a_second_failure_after_a_resume_keeps_the_first_attempts_text() {
        let mut draft = generating();
        draft.append_response("first", at(2)).unwrap();
        draft.interrupt(at(3)).unwrap();
        draft.resume(at(4)).unwrap();
        draft.append_response("second", at(5)).unwrap();
        draft.fail("timeout", at(6)).unwrap();

        draft.resume(at(7)).unwrap();

        assert_eq!(
            draft.recovered(),
            Some("second"),
            "the most recent text is the one kept"
        );
    }

    #[test]
    fn a_resume_with_no_text_keeps_what_was_recovered_before() {
        let mut draft = generating();
        draft.append_response("kept", at(2)).unwrap();
        draft.interrupt(at(3)).unwrap();
        draft.resume(at(4)).unwrap();
        draft.fail("refused", at(5)).unwrap();

        draft.resume(at(6)).unwrap();

        assert_eq!(draft.recovered(), Some("kept"));
    }

    #[test]
    fn completing_after_a_resume_drops_the_recovered_text() {
        let mut draft = generating();
        draft.append_response("partial", at(2)).unwrap();
        draft.interrupt(at(3)).unwrap();
        draft.resume(at(4)).unwrap();

        draft
            .complete(vec![section("Scope")], Vec::new(), at(5))
            .unwrap();

        assert_eq!(draft.recovered(), None);
    }

    #[test]
    fn transitions_that_dont_apply_to_the_state_are_refused() {
        let mut ready = generating();
        ready.complete(Vec::new(), Vec::new(), at(2)).unwrap();
        assert_eq!(
            ready.append_response("late", at(3)),
            Err(PlanDraftError::IllegalTransition {
                state: DraftState::Ready,
                action: "receive text"
            })
        );
        assert!(
            ready.resume(at(3)).is_err(),
            "a ready draft has nothing to resume"
        );

        let mut interrupted = generating();
        interrupted.interrupt(at(2)).unwrap();
        assert!(interrupted.complete(Vec::new(), Vec::new(), at(3)).is_err());
        assert!(
            interrupted.interrupt(at(3)).is_err(),
            "only a generating draft is interrupted"
        );
    }

    #[test]
    fn a_stored_draft_that_breaks_a_rule_is_refused_when_loaded() {
        let parts = super::PlanDraftParts {
            id: crate::ids::PlanDraftId::new(),
            project_id: ProjectId::new(),
            prompt: "p".to_string(),
            state: DraftState::Ready,
            response: String::new(),
            recovered: None,
            failure: None,
            sections: Vec::new(),
            tasks: vec![PlanTask {
                title: "t".to_string(),
                section: Some(0),
            }],
            created_at: at(1),
            updated_at: at(1),
        };
        assert_eq!(
            PlanDraft::from_parts(parts),
            Err(PlanDraftError::UnknownSection { section: 0 })
        );
    }

    #[test]
    fn states_round_trip_through_their_stored_names() {
        for state in DraftState::ALL {
            assert_eq!(state.as_str().parse::<DraftState>(), Ok(state));
        }
        assert!("paused".parse::<DraftState>().is_err());
    }
}
