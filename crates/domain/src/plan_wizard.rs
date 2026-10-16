//! The planning wizard's state: the goal, the team, and the target date the user entered, and the phase the
//! generation is in. The UI draws this and sends it events. It holds no views and no tasks.
//!
//! The inputs live here, not in the text fields, so they survive a failure. Every transition changes only the
//! phase, so a failed generation leaves the goal, the team, and the date as they were.
//!
//! Decisions this module enforces:
//! - An empty or whitespace-only goal is refused, here and not only in the UI, so no path can start a generation
//!   without one.
//! - A second start while generating is refused. The guard is the phase, not the button.
//! - Chunks, failures, and successes that arrive after a cancel, or outside the phase they belong to, are ignored.
//! - A team member who was removed while the form was open is re-checked when generation starts. Their selection is
//!   cleared, so the user picks again rather than assigning someone who no longer exists.
//! - A target date in the past is a warning, not an error.

use std::fmt;

use crate::{
    error::DomainError,
    ids::{PlanDraftId, TeamMemberId},
    task_fields::{DueDate, parse_due_date},
};

/// The longest goal, in characters after trimming. A goal is a sentence or a paragraph, not a document.
pub const MAX_GOAL_CHARS: usize = 4_000;

/// Why the wizard refused a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WizardError {
    EmptyGoal,
    GoalTooLong {
        max: usize,
    },
    /// The target date text isn't a real date. Blank is no target date, and is allowed.
    InvalidDate(String),
    /// The selected team member no longer belongs to the project.
    TeamMemberRemoved,
    /// A generation is already running.
    AlreadyGenerating,
    /// The plan is already ready.
    AlreadyDone,
}

impl fmt::Display for WizardError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WizardError::EmptyGoal => write!(f, "Describe the goal to plan from."),
            WizardError::GoalTooLong { max } => {
                write!(f, "The goal can be at most {max} characters.")
            }
            WizardError::InvalidDate(_) => {
                write!(
                    f,
                    "Enter the target date as a real day, such as 2026-10-30."
                )
            }
            WizardError::TeamMemberRemoved => write!(
                f,
                "That team member was removed from the project. Choose someone else, or no one."
            ),
            WizardError::AlreadyGenerating => write!(f, "A plan is already being generated."),
            WizardError::AlreadyDone => write!(f, "The plan is already ready."),
        }
    }
}

impl From<DomainError> for WizardError {
    fn from(err: DomainError) -> Self {
        match err {
            DomainError::InvalidDueDate(text) => WizardError::InvalidDate(text),
            other => WizardError::InvalidDate(other.to_string()),
        }
    }
}

/// Trims the goal and checks it. Blank goals are refused.
pub fn validate_goal(text: &str) -> Result<String, WizardError> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(WizardError::EmptyGoal);
    }
    if trimmed.chars().count() > MAX_GOAL_CHARS {
        return Err(WizardError::GoalTooLong {
            max: MAX_GOAL_CHARS,
        });
    }
    Ok(trimmed.to_string())
}

/// Where the generation is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WizardPhase {
    /// The inputs can be edited, and nothing is running.
    Editing,
    /// The response is arriving. `received_chars` counts the characters so far.
    Generating {
        draft: PlanDraftId,
        received_chars: usize,
    },
    /// The request ended without a usable plan. The inputs are unchanged, so the user can retry.
    Failed { message: String },
    /// The plan was read and stored.
    Done,
}

/// What a generation runs with, once the inputs have been checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Generation {
    pub goal: String,
    pub team: Option<TeamMemberId>,
    pub target: Option<DueDate>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanWizard {
    goal: String,
    team: Option<TeamMemberId>,
    target_text: String,
    today: DueDate,
    phase: WizardPhase,
}

impl PlanWizard {
    /// A wizard with an empty goal, the given team member selected, and today as the target date. `today` is
    /// passed in, so the default is the same wherever it's read.
    pub fn new(today: DueDate, team: Option<TeamMemberId>) -> Self {
        Self {
            goal: String::new(),
            team,
            target_text: today.to_string(),
            today,
            phase: WizardPhase::Editing,
        }
    }

    pub fn goal(&self) -> &str {
        &self.goal
    }

    /// Stores the goal as typed. Trimming happens when the goal is checked, so typing isn't interrupted.
    pub fn set_goal(&mut self, text: &str) {
        self.goal = text.to_string();
    }

    pub fn team(&self) -> Option<TeamMemberId> {
        self.team
    }

    pub fn set_team(&mut self, team: Option<TeamMemberId>) {
        self.team = team;
    }

    pub fn target_text(&self) -> &str {
        &self.target_text
    }

    pub fn set_target_text(&mut self, text: &str) {
        self.target_text = text.to_string();
    }

    pub fn phase(&self) -> &WizardPhase {
        &self.phase
    }

    /// A wizard showing a plan that's already ready. Used for a draft reopened from the list, which shows the review
    /// straight away.
    pub fn finished(today: DueDate) -> Self {
        let mut wizard = Self::new(today, None);
        wizard.phase = WizardPhase::Done;
        wizard
    }

    /// The goal's problem, when it has one. Shown next to the field.
    pub fn goal_problem(&self) -> Option<WizardError> {
        validate_goal(&self.goal).err()
    }

    /// The target date as typed, when it's a real day. Blank is no date, and a date that doesn't parse is an
    /// error.
    pub fn target_date(&self) -> Result<Option<DueDate>, WizardError> {
        Ok(parse_due_date(&self.target_text)?)
    }

    /// Whether the target date is before today. Shown as a warning, since a past date can still be a real plan.
    pub fn target_is_past(&self) -> bool {
        matches!(self.target_date(), Ok(Some(date)) if date < self.today)
    }

    /// Whether Generate can be used now: the goal and date are valid, and nothing is running or done.
    pub fn can_generate(&self) -> bool {
        matches!(
            self.phase,
            WizardPhase::Editing | WizardPhase::Failed { .. }
        ) && self.goal_problem().is_none()
            && self.target_date().is_ok()
    }

    /// Checks the inputs and moves to `Generating`, for the given draft. `members` is the project's current team,
    /// read now, so a member removed since the form opened is caught here.
    ///
    /// Nothing changes when a check fails, except the team selection, which is cleared when its member is gone.
    pub fn begin(
        &mut self,
        members: &[TeamMemberId],
        draft: PlanDraftId,
    ) -> Result<Generation, WizardError> {
        match self.phase {
            WizardPhase::Generating { .. } => return Err(WizardError::AlreadyGenerating),
            WizardPhase::Done => return Err(WizardError::AlreadyDone),
            WizardPhase::Editing | WizardPhase::Failed { .. } => {}
        }
        let goal = validate_goal(&self.goal)?;
        let target = self.target_date()?;
        if let Some(team) = self.team
            && !members.contains(&team)
        {
            self.team = None;
            return Err(WizardError::TeamMemberRemoved);
        }
        self.phase = WizardPhase::Generating {
            draft,
            received_chars: 0,
        };
        Ok(Generation {
            goal,
            team: self.team,
            target,
        })
    }

    /// Counts characters that arrived. Ignored unless a generation is running.
    pub fn receive(&mut self, chars: usize) -> bool {
        match &mut self.phase {
            WizardPhase::Generating { received_chars, .. } => {
                *received_chars += chars;
                true
            }
            _ => false,
        }
    }

    /// Stops a running generation and returns to editing, with the inputs untouched. Returns the draft that was
    /// running, so the caller can record the cancel. `None` when nothing was running.
    pub fn cancel(&mut self) -> Option<PlanDraftId> {
        match self.phase {
            WizardPhase::Generating { draft, .. } => {
                self.phase = WizardPhase::Editing;
                Some(draft)
            }
            _ => None,
        }
    }

    /// The request failed or the reply couldn't be read. Only the phase changes.
    pub fn fail(&mut self, message: &str) -> bool {
        match self.phase {
            WizardPhase::Generating { .. } => {
                self.phase = WizardPhase::Failed {
                    message: message.to_string(),
                };
                true
            }
            _ => false,
        }
    }

    /// The reply was read and stored.
    pub fn succeed(&mut self) -> bool {
        match self.phase {
            WizardPhase::Generating { .. } => {
                self.phase = WizardPhase::Done;
                true
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    // Not `use super::*`: gpui's `test` macro would shadow `#[test]`.
    use crate::{
        ids::{PlanDraftId, TeamMemberId},
        plan_wizard::{PlanWizard, WizardError, WizardPhase, validate_goal},
        task_fields::DueDate,
    };

    fn today() -> DueDate {
        DueDate::new(2026, 10, 17).unwrap()
    }

    fn wizard_with_goal(goal: &str) -> PlanWizard {
        let mut wizard = PlanWizard::new(today(), None);
        wizard.set_goal(goal);
        wizard
    }

    #[test]
    fn an_empty_goal_is_refused_and_a_whitespace_goal_is_too() {
        assert_eq!(validate_goal(""), Err(WizardError::EmptyGoal));
        assert_eq!(validate_goal("   \n\t "), Err(WizardError::EmptyGoal));
        assert_eq!(validate_goal("  Ship it  "), Ok("Ship it".to_string()));
    }

    #[test]
    fn a_goal_that_is_too_long_is_refused() {
        let long = "x".repeat(super::MAX_GOAL_CHARS + 1);
        assert_eq!(
            validate_goal(&long),
            Err(WizardError::GoalTooLong {
                max: super::MAX_GOAL_CHARS
            })
        );
    }

    #[test]
    fn an_empty_goal_never_starts_a_generation_and_the_phase_stays_editing() {
        let mut wizard = wizard_with_goal("   ");
        let draft = PlanDraftId::new();
        assert_eq!(
            wizard.begin(&[], draft),
            Err(WizardError::EmptyGoal),
            "the check is in the domain, not only in the button"
        );
        assert_eq!(wizard.phase(), &WizardPhase::Editing);
        assert!(!wizard.can_generate());
    }

    #[test]
    fn a_second_start_while_generating_does_nothing() {
        let mut wizard = wizard_with_goal("Plan the launch");
        wizard.begin(&[], PlanDraftId::new()).unwrap();

        assert_eq!(
            wizard.begin(&[], PlanDraftId::new()),
            Err(WizardError::AlreadyGenerating)
        );
    }

    #[test]
    fn a_failure_keeps_every_input_and_changes_only_the_phase() {
        let member = TeamMemberId::new();
        let mut wizard = PlanWizard::new(today(), Some(member));
        wizard.set_goal("Plan the launch");
        wizard.set_target_text("2026-10-30");
        wizard.begin(&[member], PlanDraftId::new()).unwrap();

        assert!(wizard.fail("the connection dropped"));

        assert_eq!(wizard.goal(), "Plan the launch");
        assert_eq!(wizard.team(), Some(member));
        assert_eq!(wizard.target_text(), "2026-10-30");
        assert_eq!(
            wizard.phase(),
            &WizardPhase::Failed {
                message: "the connection dropped".to_string()
            }
        );
    }

    #[test]
    fn a_retry_after_a_failure_starts_with_the_same_inputs() {
        let mut wizard = wizard_with_goal("Plan the launch");
        wizard.begin(&[], PlanDraftId::new()).unwrap();
        wizard.fail("timeout");

        let generation = wizard.begin(&[], PlanDraftId::new()).unwrap();

        assert_eq!(generation.goal, "Plan the launch");
    }

    #[test]
    fn cancelling_returns_to_editing_with_the_inputs_and_names_the_draft() {
        let mut wizard = wizard_with_goal("Plan the launch");
        let draft = PlanDraftId::new();
        wizard.begin(&[], draft).unwrap();

        assert_eq!(wizard.cancel(), Some(draft));
        assert_eq!(wizard.phase(), &WizardPhase::Editing);
        assert_eq!(wizard.goal(), "Plan the launch");
        assert_eq!(wizard.cancel(), None, "a second cancel has nothing to stop");
    }

    #[test]
    fn chunks_after_a_cancel_are_ignored() {
        let mut wizard = wizard_with_goal("Plan");
        wizard.begin(&[], PlanDraftId::new()).unwrap();
        wizard.receive(10);
        wizard.cancel();

        assert!(!wizard.receive(5), "a late chunk is not counted");
        assert!(!wizard.fail("late error"), "a late failure changes nothing");
        assert!(!wizard.succeed(), "a late success changes nothing");
        assert_eq!(wizard.phase(), &WizardPhase::Editing);
    }

    #[test]
    fn received_characters_are_counted_while_generating() {
        let mut wizard = wizard_with_goal("Plan");
        wizard.begin(&[], PlanDraftId::new()).unwrap();
        wizard.receive(4);
        wizard.receive(6);

        assert!(matches!(
            wizard.phase(),
            WizardPhase::Generating {
                received_chars: 10,
                ..
            }
        ));
    }

    #[test]
    fn a_removed_team_member_is_refused_at_start_and_the_selection_is_cleared() {
        let member = TeamMemberId::new();
        let mut wizard = PlanWizard::new(today(), Some(member));
        wizard.set_goal("Plan");

        assert_eq!(
            wizard.begin(&[], PlanDraftId::new()),
            Err(WizardError::TeamMemberRemoved)
        );
        assert_eq!(wizard.team(), None);
        assert_eq!(wizard.phase(), &WizardPhase::Editing);
    }

    #[test]
    fn a_team_member_still_on_the_project_is_kept() {
        let member = TeamMemberId::new();
        let mut wizard = PlanWizard::new(today(), Some(member));
        wizard.set_goal("Plan");

        let generation = wizard.begin(&[member], PlanDraftId::new()).unwrap();
        assert_eq!(generation.team, Some(member));
    }

    #[test]
    fn a_blank_target_is_no_date_and_a_bad_one_blocks_generation() {
        let mut wizard = wizard_with_goal("Plan");
        wizard.set_target_text("");
        assert_eq!(wizard.target_date(), Ok(None));
        assert!(wizard.can_generate());

        wizard.set_target_text("next friday");
        assert!(matches!(
            wizard.target_date(),
            Err(WizardError::InvalidDate(_))
        ));
        assert!(!wizard.can_generate());
        assert!(wizard.begin(&[], PlanDraftId::new()).is_err());
    }

    #[test]
    fn a_past_target_is_a_warning_not_a_block() {
        let mut wizard = wizard_with_goal("Plan");
        wizard.set_target_text("2026-10-01");
        assert!(wizard.target_is_past());
        assert!(wizard.can_generate(), "a past date still generates");
        assert!(wizard.begin(&[], PlanDraftId::new()).is_ok());
    }

    #[test]
    fn the_default_target_is_the_day_the_wizard_was_given() {
        let wizard = PlanWizard::new(today(), None);
        assert_eq!(wizard.target_text(), "2026-10-17");
        assert!(!wizard.target_is_past());
    }

    #[test]
    fn a_finished_plan_cannot_be_generated_again_from_the_same_wizard() {
        let mut wizard = wizard_with_goal("Plan");
        wizard.begin(&[], PlanDraftId::new()).unwrap();
        assert!(wizard.succeed());

        assert_eq!(
            wizard.begin(&[], PlanDraftId::new()),
            Err(WizardError::AlreadyDone)
        );
    }
}
