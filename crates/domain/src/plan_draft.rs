//! Planning drafts: what a planning request produced, kept so it can be reopened, resumed, reviewed, and edited.
//!
//! The states:
//! - `Generating`: a request is in flight. Text is arriving.
//! - `Interrupted`: the app closed during generation. A generation can't outlive the process, so a draft still
//!   generating when the store next opens is interrupted.
//! - `Failed`: the request ended with an error, and the reason is kept.
//! - `Ready`: the response was read, and the spec, user stories, and tasks are here to review and edit.
//!
//! Partial text is never discarded. An interrupted or failed draft keeps the text it received, since a partial
//! response can still hold a plan worth recovering. Resuming starts a new generation and moves the old text to
//! `recovered`, so a second failure doesn't lose it. A completed draft has nothing to recover, so it drops it.
//! Discarding deletes the draft; that's the repository's job, not a state here.
//!
//! The text is kept as it was received. This module doesn't parse it. The strict JSON parser turns the text into
//! the spec, stories, and tasks, and `complete` is where those go in.
//!
//! Once a draft is ready, it's edited one change at a time with `PlanEdit`. Each change is checked against every
//! rule before it's applied, and a refused change leaves the draft as it was. A ready draft may have no tasks at
//! all: deleting every task is a normal state with an empty backlog, not an error.

use std::{fmt, str::FromStr};

use thiserror::Error;

use crate::{
    error::DomainError,
    ids::{PlanDraftId, ProjectId, TaskId, TeamMemberId},
    task::Estimate,
    task_fields::{Priority, TagError, normalize_tags},
    time::Timestamp,
    validation::MAX_TEXT_CHARS,
};

/// Why a draft refused a change, or a value in it. Each variant names the rule.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PlanDraftError {
    #[error("a planning request needs a prompt")]
    EmptyPrompt,
    #[error("the spec can't be empty")]
    EmptySpec,
    #[error("a user story needs a title")]
    EmptyStoryTitle,
    #[error("a task needs a title")]
    EmptyTaskTitle,
    #[error("a title can be at most {max} characters")]
    TitleTooLong { max: usize },
    #[error("a task refers to story {section}, which doesn't exist")]
    UnknownStory { section: usize },
    #[error("task {index} repeats the title of task {first}, ignoring case")]
    DuplicateTitle { index: usize, first: usize },
    #[error("there is no item {index} to change; there are {len}")]
    IndexOutOfRange { index: usize, len: usize },
    #[error("that team member doesn't belong to this project")]
    UnknownAssignee,
    #[error("this was edited by hand; replacing it needs confirmation")]
    ManuallyEdited,
    #[error("this plan has been published, so it can't change")]
    AlreadyPublished,
    #[error("that refinement is for a different part of the plan")]
    WrongPart,
    #[error("{0}")]
    Tag(TagError),
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

/// One user story. Its title and body are what the plan said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanStory {
    pub title: String,
    pub body: String,
    /// Whether a person wrote or changed this story, rather than the plan. Refinement asks before replacing it.
    pub edited: bool,
}

/// A task in the backlog review. Only the title is required, and everything else can be left empty.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanTask {
    pub title: String,
    pub priority: Priority,
    pub estimate: Option<Estimate>,
    pub assignee: Option<TeamMemberId>,
    pub tags: Vec<String>,
    /// The index of the story this task belongs to, if any.
    pub section: Option<usize>,
    /// Whether a person wrote or changed this task, rather than the plan. Refinement asks before replacing it.
    pub edited: bool,
}

impl PlanTask {
    /// A task with a title and the defaults for everything else: medium priority, no estimate, unassigned, and no
    /// tags. Used when a user adds one.
    pub fn new(title: &str) -> Result<Self, PlanDraftError> {
        Ok(Self {
            title: clean_title(title, PlanDraftError::EmptyTaskTitle)?,
            priority: Priority::Medium,
            estimate: None,
            assignee: None,
            tags: Vec::new(),
            section: None,
            edited: false,
        })
    }
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
    pub spec: String,
    pub spec_edited: bool,
    pub stories: Vec<PlanStory>,
    pub tasks: Vec<PlanTask>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub published_at: Option<Timestamp>,
}

/// One change to a ready draft. Every change goes through `PlanDraft::apply`, so each one is checked the same way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanEdit {
    SetSpec(String),
    SetStoryTitle {
        index: usize,
        title: String,
    },
    SetStoryBody {
        index: usize,
        body: String,
    },
    RemoveStory(usize),
    SetTaskTitle {
        index: usize,
        title: String,
    },
    SetTaskPriority {
        index: usize,
        priority: Priority,
    },
    SetTaskEstimate {
        index: usize,
        estimate: Option<Estimate>,
    },
    /// Must be one of the project's team members, or none.
    SetTaskAssignee {
        index: usize,
        assignee: Option<TeamMemberId>,
    },
    /// The tags as typed. They're normalized and checked when the change is applied.
    SetTaskTags {
        index: usize,
        tags: Vec<String>,
    },
    AddTask {
        title: String,
    },
    RemoveTask(usize),
    /// Moves the task at `from` so it ends up at `to`. Both are indexes in the current list.
    MoveTask {
        from: usize,
        to: usize,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanDraft {
    parts: PlanDraftParts,
}

/// Trims a title, and checks it's not blank and not over the limit.
fn clean_title(title: &str, empty: PlanDraftError) -> Result<String, PlanDraftError> {
    let trimmed = title.trim();
    if trimmed.is_empty() {
        return Err(empty);
    }
    if trimmed.chars().count() > MAX_TEXT_CHARS {
        return Err(PlanDraftError::TitleTooLong {
            max: MAX_TEXT_CHARS,
        });
    }
    Ok(trimmed.to_string())
}

/// Trims a spec, and checks it's not blank.
fn clean_spec(spec: &str) -> Result<String, PlanDraftError> {
    let trimmed = spec.trim();
    if trimmed.is_empty() {
        return Err(PlanDraftError::EmptySpec);
    }
    Ok(trimmed.to_string())
}

/// Trims a goal, and checks it's not blank.
fn clean_prompt(prompt: &str) -> Result<String, PlanDraftError> {
    let trimmed = prompt.trim();
    if trimmed.is_empty() {
        return Err(PlanDraftError::EmptyPrompt);
    }
    Ok(trimmed.to_string())
}

/// Checks the content of a draft as a whole: story titles, task titles, the stories tasks refer to, and that no two
/// tasks repeat a title, ignoring case. A draft with no tasks passes, since an empty backlog is valid.
fn check_content(stories: &[PlanStory], tasks: &[PlanTask]) -> Result<(), PlanDraftError> {
    for story in stories {
        clean_title(&story.title, PlanDraftError::EmptyStoryTitle)?;
    }
    let mut seen: Vec<(String, usize)> = Vec::new();
    for (index, task) in tasks.iter().enumerate() {
        let title = clean_title(&task.title, PlanDraftError::EmptyTaskTitle)?;
        if let Some(section) = task.section
            && section >= stories.len()
        {
            return Err(PlanDraftError::UnknownStory { section });
        }
        let key = title.to_lowercase();
        match seen.iter().find(|(known, _)| *known == key) {
            Some((_, first)) => {
                return Err(PlanDraftError::DuplicateTitle {
                    index,
                    first: *first,
                });
            }
            None => seen.push((key, index)),
        }
    }
    Ok(())
}

fn check_index(index: usize, len: usize) -> Result<(), PlanDraftError> {
    if index < len {
        Ok(())
    } else {
        Err(PlanDraftError::IndexOutOfRange { index, len })
    }
}

impl PlanDraft {
    /// A new draft for a goal. It starts generating, and nothing has been received yet.
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
                spec: String::new(),
                spec_edited: false,
                stories: Vec::new(),
                tasks: Vec::new(),
                created_at: at,
                updated_at: at,
                published_at: None,
            },
        })
    }

    /// Rebuilds a draft from storage. Checks what the transitions check, so a stored draft that breaks a rule is
    /// refused rather than loaded. A ready draft must have a spec.
    pub fn from_parts(parts: PlanDraftParts) -> Result<Self, PlanDraftError> {
        let prompt = clean_prompt(&parts.prompt)?;
        let spec = if parts.state == DraftState::Ready {
            clean_spec(&parts.spec)?
        } else {
            parts.spec
        };
        check_content(&parts.stories, &parts.tasks)?;
        Ok(Self {
            parts: PlanDraftParts {
                prompt,
                spec,
                ..parts
            },
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

    pub fn spec(&self) -> &str {
        &self.parts.spec
    }

    pub fn stories(&self) -> &[PlanStory] {
        &self.parts.stories
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
    /// an earlier attempt is dropped, since the plan now stands on its own. A plan needs a spec, but it may have
    /// no tasks.
    pub fn complete(
        &mut self,
        spec: &str,
        stories: Vec<PlanStory>,
        tasks: Vec<PlanTask>,
        at: Timestamp,
    ) -> Result<(), PlanDraftError> {
        self.require(DraftState::Generating, "complete")?;
        let spec = clean_spec(spec)?;
        check_content(&stories, &tasks)?;
        self.parts.spec = spec;
        self.parts.stories = stories;
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

    /// Applies one change to a ready draft. `members` is the project's current team, so an assignee outside it is
    /// refused.
    ///
    /// The change is applied to a copy and checked as a whole. A refused change leaves the draft exactly as it was.
    pub fn apply(
        &mut self,
        edit: PlanEdit,
        members: &[TeamMemberId],
        at: Timestamp,
    ) -> Result<(), PlanDraftError> {
        self.require_unpublished("be edited")?;
        let mut next = self.parts.clone();
        apply_to(&mut next, edit, members)?;
        check_content(&next.stories, &next.tasks)?;
        next.updated_at = at;
        self.parts = next;
        Ok(())
    }

    /// The published flag: when the plan was published, or `None` if it hasn't been.
    pub fn published_at(&self) -> Option<Timestamp> {
        self.parts.published_at
    }

    /// The tasks a publication would create, and the titles it would skip because the project already has a task with
    /// that title, ignoring case. Requires a ready plan that hasn't been published.
    pub fn publication_plan(
        &self,
        existing_titles: &[String],
    ) -> Result<PublishPlan, PlanDraftError> {
        self.require_unpublished("be published")?;
        let taken: Vec<String> = existing_titles
            .iter()
            .map(|title| title.trim().to_lowercase())
            .collect();
        let mut plan = PublishPlan::default();
        for task in &self.parts.tasks {
            if taken.contains(&task.title.trim().to_lowercase()) {
                plan.skipped.push(task.title.clone());
            } else {
                plan.create.push(task.clone());
            }
        }
        Ok(plan)
    }

    /// Records that the plan was published. After this the plan can't change. Only a ready plan that hasn't been
    /// published can be marked.
    pub fn mark_published(&mut self, at: Timestamp) -> Result<(), PlanDraftError> {
        self.require_unpublished("be published")?;
        self.parts.published_at = Some(at);
        self.parts.updated_at = at;
        Ok(())
    }

    /// A ready plan that hasn't been published. Only this kind of plan can change.
    fn require_unpublished(&self, action: &'static str) -> Result<(), PlanDraftError> {
        self.require(DraftState::Ready, action)?;
        if self.parts.published_at.is_some() {
            return Err(PlanDraftError::AlreadyPublished);
        }
        Ok(())
    }

    /// A short name for the draft: the first line of its spec, or of its goal before there is one. Headings' `#` marks
    /// are dropped, and the name is cut to 80 characters.
    pub fn title(&self) -> String {
        let source = if self.parts.spec.trim().is_empty() {
            &self.parts.prompt
        } else {
            &self.parts.spec
        };
        source
            .lines()
            .map(|line| line.trim().trim_start_matches('#').trim())
            .find(|line| !line.is_empty())
            .unwrap_or("Untitled plan")
            .chars()
            .take(80)
            .collect()
    }

    /// A copy of this draft under a new id. A ready plan copies its content and is unpublished, so the copy can be
    /// edited and published on its own. Any other draft copies its goal only, as an interrupted draft, so the copy
    /// can be resumed.
    pub fn duplicate(&self, id: PlanDraftId, at: Timestamp) -> Result<PlanDraft, PlanDraftError> {
        if self.parts.state == DraftState::Ready {
            let mut parts = self.parts.clone();
            parts.id = id;
            parts.response = String::new();
            parts.recovered = None;
            parts.failure = None;
            parts.published_at = None;
            parts.created_at = at;
            parts.updated_at = at;
            return PlanDraft::from_parts(parts);
        }
        let mut copy = PlanDraft::start_as(id, self.parts.project_id, &self.parts.prompt, at)?;
        copy.interrupt(at)?;
        Ok(copy)
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

/// Applies one change to the parts. The caller checks the whole result.
fn apply_to(
    parts: &mut PlanDraftParts,
    edit: PlanEdit,
    members: &[TeamMemberId],
) -> Result<(), PlanDraftError> {
    match edit {
        PlanEdit::SetSpec(spec) => {
            parts.spec = clean_spec(&spec)?;
            parts.spec_edited = true;
        }
        PlanEdit::SetStoryTitle { index, title } => {
            check_index(index, parts.stories.len())?;
            parts.stories[index].title = clean_title(&title, PlanDraftError::EmptyStoryTitle)?;
            parts.stories[index].edited = true;
        }
        PlanEdit::SetStoryBody { index, body } => {
            check_index(index, parts.stories.len())?;
            parts.stories[index].body = body.trim().to_string();
            parts.stories[index].edited = true;
        }
        PlanEdit::RemoveStory(index) => {
            check_index(index, parts.stories.len())?;
            parts.stories.remove(index);
            // A task that belonged to the removed story is left without one. Tasks after it keep their story.
            for task in &mut parts.tasks {
                task.section = match task.section {
                    Some(section) if section == index => None,
                    Some(section) if section > index => Some(section - 1),
                    other => other,
                };
            }
        }
        PlanEdit::SetTaskTitle { index, title } => {
            check_index(index, parts.tasks.len())?;
            parts.tasks[index].title = clean_title(&title, PlanDraftError::EmptyTaskTitle)?;
            parts.tasks[index].edited = true;
        }
        PlanEdit::SetTaskPriority { index, priority } => {
            check_index(index, parts.tasks.len())?;
            parts.tasks[index].priority = priority;
            parts.tasks[index].edited = true;
        }
        PlanEdit::SetTaskEstimate { index, estimate } => {
            check_index(index, parts.tasks.len())?;
            parts.tasks[index].estimate = estimate;
            parts.tasks[index].edited = true;
        }
        PlanEdit::SetTaskAssignee { index, assignee } => {
            check_index(index, parts.tasks.len())?;
            if let Some(member) = assignee
                && !members.contains(&member)
            {
                return Err(PlanDraftError::UnknownAssignee);
            }
            parts.tasks[index].assignee = assignee;
            parts.tasks[index].edited = true;
        }
        PlanEdit::SetTaskTags { index, tags } => {
            check_index(index, parts.tasks.len())?;
            let tags =
                normalize_tags(tags.iter().map(String::as_str)).map_err(PlanDraftError::Tag)?;
            parts.tasks[index].tags = tags;
            parts.tasks[index].edited = true;
        }
        PlanEdit::AddTask { title } => {
            let mut task = PlanTask::new(&title)?;
            task.edited = true;
            parts.tasks.push(task);
        }
        PlanEdit::RemoveTask(index) => {
            check_index(index, parts.tasks.len())?;
            parts.tasks.remove(index);
        }
        PlanEdit::MoveTask { from, to } => {
            check_index(from, parts.tasks.len())?;
            check_index(to, parts.tasks.len())?;
            let task = parts.tasks.remove(from);
            parts.tasks.insert(to, task);
        }
    }
    Ok(())
}

/// Which part of a ready plan a refinement replaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefineTarget {
    Spec,
    Story(usize),
    Task(usize),
}

/// The replacement a refinement produced. It has to match its target: a spec for the spec, a story for a story,
/// and a task for a task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefinedPart {
    Spec(String),
    Story(PlanStory),
    Task(PlanTask),
}

impl PlanDraft {
    /// Replaces one part of a ready plan, and leaves every other part as it was.
    ///
    /// A part a person edited by hand is replaced only when `replace_edited` is set. Otherwise the change is refused
    /// with `ManuallyEdited`, so the caller can ask first. The replacement is generated content again, so it isn't
    /// marked edited. A task keeps its story, since a refinement doesn't move it.
    ///
    /// The change is checked as a whole, like every edit, and a refused refinement leaves the draft as it was.
    pub fn refine(
        &mut self,
        target: RefineTarget,
        part: RefinedPart,
        members: &[TeamMemberId],
        replace_edited: bool,
        at: Timestamp,
    ) -> Result<(), PlanDraftError> {
        self.require_unpublished("be refined")?;
        let mut next = self.parts.clone();
        match (target, part) {
            (RefineTarget::Spec, RefinedPart::Spec(spec)) => {
                if next.spec_edited && !replace_edited {
                    return Err(PlanDraftError::ManuallyEdited);
                }
                next.spec = clean_spec(&spec)?;
                next.spec_edited = false;
            }
            (RefineTarget::Story(index), RefinedPart::Story(story)) => {
                check_index(index, next.stories.len())?;
                if next.stories[index].edited && !replace_edited {
                    return Err(PlanDraftError::ManuallyEdited);
                }
                next.stories[index] = PlanStory {
                    title: clean_title(&story.title, PlanDraftError::EmptyStoryTitle)?,
                    body: story.body.trim().to_string(),
                    edited: false,
                };
            }
            (RefineTarget::Task(index), RefinedPart::Task(task)) => {
                check_index(index, next.tasks.len())?;
                if next.tasks[index].edited && !replace_edited {
                    return Err(PlanDraftError::ManuallyEdited);
                }
                if let Some(member) = task.assignee
                    && !members.contains(&member)
                {
                    return Err(PlanDraftError::UnknownAssignee);
                }
                let tags = normalize_tags(task.tags.iter().map(String::as_str))
                    .map_err(PlanDraftError::Tag)?;
                let section = next.tasks[index].section;
                next.tasks[index] = PlanTask {
                    title: clean_title(&task.title, PlanDraftError::EmptyTaskTitle)?,
                    priority: task.priority,
                    estimate: task.estimate,
                    assignee: task.assignee,
                    tags,
                    section,
                    edited: false,
                };
            }
            _ => return Err(PlanDraftError::WrongPart),
        }
        check_content(&next.stories, &next.tasks)?;
        next.updated_at = at;
        self.parts = next;
        Ok(())
    }

    pub fn spec_edited(&self) -> bool {
        self.parts.spec_edited
    }
}

/// The tasks a publication would create, and the titles it would skip. Every task of the plan is in one of the two
/// lists, in the order the plan has them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PublishPlan {
    pub create: Vec<PlanTask>,
    /// Titles the project already has, ignoring case. Those tasks aren't created again.
    pub skipped: Vec<String>,
}

/// What a publication did: the new tasks, and the titles it skipped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Publication {
    pub published: Vec<TaskId>,
    pub skipped: Vec<String>,
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
        ids::{ProjectId, TeamMemberId},
        plan_draft::{
            DraftState, PlanDraft, PlanDraftError, PlanDraftParts, PlanEdit, PlanStory, PlanTask,
            RefineTarget, RefinedPart,
        },
        task::Estimate,
        task_fields::Priority,
        time::Timestamp,
    };

    fn at(millis: i64) -> Timestamp {
        Timestamp::from_unix_millis(millis)
    }

    fn generating() -> PlanDraft {
        PlanDraft::start(ProjectId::new(), "  Plan the launch  ", at(1)).unwrap()
    }

    fn story(title: &str) -> PlanStory {
        PlanStory {
            title: title.to_string(),
            body: format!("About {title}"),
            edited: false,
        }
    }

    fn task(title: &str) -> PlanTask {
        PlanTask::new(title).unwrap()
    }

    fn ready_with(tasks: Vec<PlanTask>) -> PlanDraft {
        let mut draft = generating();
        draft
            .complete("The spec.", vec![story("Scope")], tasks, at(2))
            .unwrap();
        draft
    }

    fn titles(draft: &PlanDraft) -> Vec<&str> {
        draft
            .tasks()
            .iter()
            .map(|task| task.title.as_str())
            .collect()
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
        draft.append_response("{\"spec\":[", at(2)).unwrap();
        draft.append_response("]}", at(3)).unwrap();
        assert_eq!(draft.response(), "{\"spec\":[]}");
        assert_eq!(draft.updated_at(), at(3));
    }

    #[test]
    fn completing_stores_the_spec_stories_and_tasks_and_moves_to_ready() {
        let mut draft = generating();
        draft
            .complete(
                "  The spec.  ",
                vec![story("Scope")],
                vec![task("Write the brief")],
                at(4),
            )
            .unwrap();
        assert_eq!(draft.state(), DraftState::Ready);
        assert_eq!(draft.spec(), "The spec.");
        assert_eq!(draft.stories().len(), 1);
        assert_eq!(draft.tasks()[0].priority, Priority::Medium);
    }

    #[test]
    fn a_blank_spec_is_refused_when_completing() {
        let mut draft = generating();
        assert_eq!(
            draft.complete("  ", Vec::new(), Vec::new(), at(4)),
            Err(PlanDraftError::EmptySpec)
        );
        assert_eq!(draft.state(), DraftState::Generating, "nothing changed");
    }

    #[test]
    fn a_completed_plan_may_have_no_tasks() {
        let mut draft = generating();
        draft
            .complete("Spec", Vec::new(), Vec::new(), at(4))
            .unwrap();
        assert!(draft.tasks().is_empty());
    }

    #[test]
    fn a_task_in_a_story_that_doesnt_exist_is_refused() {
        let mut draft = generating();
        let mut orphan = task("Orphan");
        orphan.section = Some(3);
        assert_eq!(
            draft.complete("Spec", vec![story("Scope")], vec![orphan], at(4)),
            Err(PlanDraftError::UnknownStory { section: 3 })
        );
    }

    #[test]
    fn two_titles_that_differ_only_in_case_are_refused_when_completing() {
        let mut draft = generating();
        assert_eq!(
            draft.complete(
                "Spec",
                Vec::new(),
                vec![task("Write the note"), task("write the NOTE")],
                at(4)
            ),
            Err(PlanDraftError::DuplicateTitle { index: 1, first: 0 })
        );
    }

    #[test]
    fn a_failure_keeps_the_text_received_and_its_reason() {
        let mut draft = generating();
        draft
            .append_response("{\"spec\":[{\"title\":\"Sc", at(2))
            .unwrap();
        draft.fail("the connection dropped", at(3)).unwrap();
        assert_eq!(draft.state(), DraftState::Failed);
        assert_eq!(draft.response(), "{\"spec\":[{\"title\":\"Sc");
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
            .complete("Spec", Vec::new(), Vec::new(), at(5))
            .unwrap();

        assert_eq!(draft.recovered(), None);
    }

    #[test]
    fn transitions_that_dont_apply_to_the_state_are_refused() {
        let mut ready = generating();
        ready
            .complete("Spec", Vec::new(), Vec::new(), at(2))
            .unwrap();
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
        assert!(
            interrupted
                .complete("Spec", Vec::new(), Vec::new(), at(3))
                .is_err()
        );
        assert!(
            interrupted.interrupt(at(3)).is_err(),
            "only a generating draft is interrupted"
        );
    }

    #[test]
    fn a_stored_draft_that_breaks_a_rule_is_refused_when_loaded() {
        let parts = PlanDraftParts {
            id: crate::ids::PlanDraftId::new(),
            project_id: ProjectId::new(),
            prompt: "p".to_string(),
            state: DraftState::Ready,
            response: String::new(),
            recovered: None,
            failure: None,
            spec: "Spec".to_string(),
            spec_edited: false,
            published_at: None,
            stories: Vec::new(),
            tasks: vec![PlanTask {
                section: Some(0),
                ..task("t")
            }],
            created_at: at(1),
            updated_at: at(1),
        };
        assert_eq!(
            PlanDraft::from_parts(parts),
            Err(PlanDraftError::UnknownStory { section: 0 })
        );
    }

    #[test]
    fn a_ready_draft_with_a_blank_spec_is_refused_when_loaded() {
        let parts = PlanDraftParts {
            id: crate::ids::PlanDraftId::new(),
            project_id: ProjectId::new(),
            prompt: "p".to_string(),
            state: DraftState::Ready,
            response: String::new(),
            recovered: None,
            failure: None,
            spec: "  ".to_string(),
            spec_edited: false,
            published_at: None,
            stories: Vec::new(),
            tasks: Vec::new(),
            created_at: at(1),
            updated_at: at(1),
        };
        assert_eq!(PlanDraft::from_parts(parts), Err(PlanDraftError::EmptySpec));
    }

    #[test]
    fn editing_a_task_title_changes_it_and_is_checked_as_a_whole() {
        let mut draft = ready_with(vec![task("One"), task("Two")]);

        draft
            .apply(
                PlanEdit::SetTaskTitle {
                    index: 0,
                    title: "  Uno  ".to_string(),
                },
                &[],
                at(10),
            )
            .unwrap();
        assert_eq!(titles(&draft), vec!["Uno", "Two"]);
        assert_eq!(draft.updated_at(), at(10));

        let refused = draft.apply(
            PlanEdit::SetTaskTitle {
                index: 1,
                title: "UNO".to_string(),
            },
            &[],
            at(11),
        );
        assert_eq!(
            refused,
            Err(PlanDraftError::DuplicateTitle { index: 1, first: 0 })
        );
        assert_eq!(
            titles(&draft),
            vec!["Uno", "Two"],
            "a refused edit changes nothing"
        );
        assert_eq!(draft.updated_at(), at(10));
    }

    #[test]
    fn a_blank_title_is_refused_for_a_task_and_for_a_story() {
        let mut draft = ready_with(vec![task("One")]);
        assert_eq!(
            draft.apply(
                PlanEdit::SetTaskTitle {
                    index: 0,
                    title: "  ".to_string()
                },
                &[],
                at(3)
            ),
            Err(PlanDraftError::EmptyTaskTitle)
        );
        assert_eq!(
            draft.apply(
                PlanEdit::SetStoryTitle {
                    index: 0,
                    title: "".to_string()
                },
                &[],
                at(3)
            ),
            Err(PlanDraftError::EmptyStoryTitle)
        );
    }

    #[test]
    fn priority_estimate_and_tags_are_saved_and_tags_are_normalized() {
        let mut draft = ready_with(vec![task("One")]);
        draft
            .apply(
                PlanEdit::SetTaskPriority {
                    index: 0,
                    priority: Priority::High,
                },
                &[],
                at(3),
            )
            .unwrap();
        draft
            .apply(
                PlanEdit::SetTaskEstimate {
                    index: 0,
                    estimate: Some(Estimate::from_hours(2.5).unwrap()),
                },
                &[],
                at(4),
            )
            .unwrap();
        draft
            .apply(
                PlanEdit::SetTaskTags {
                    index: 0,
                    tags: vec!["UI".to_string(), "ui".to_string(), "docs".to_string()],
                },
                &[],
                at(5),
            )
            .unwrap();

        let saved = &draft.tasks()[0];
        assert_eq!(saved.priority, Priority::High);
        assert_eq!(saved.estimate.map(|e| e.minutes()), Some(150));
        assert_eq!(saved.tags, vec!["docs", "ui"]);
    }

    #[test]
    fn an_assignee_outside_the_project_is_refused() {
        let member = TeamMemberId::new();
        let outsider = TeamMemberId::new();
        let mut draft = ready_with(vec![task("One")]);

        draft
            .apply(
                PlanEdit::SetTaskAssignee {
                    index: 0,
                    assignee: Some(member),
                },
                &[member],
                at(3),
            )
            .unwrap();
        assert_eq!(draft.tasks()[0].assignee, Some(member));

        assert_eq!(
            draft.apply(
                PlanEdit::SetTaskAssignee {
                    index: 0,
                    assignee: Some(outsider)
                },
                &[member],
                at(4)
            ),
            Err(PlanDraftError::UnknownAssignee)
        );
        assert_eq!(draft.tasks()[0].assignee, Some(member));
    }

    #[test]
    fn removing_a_task_keeps_the_others_in_order() {
        let mut draft = ready_with(vec![task("One"), task("Two"), task("Three")]);
        draft.apply(PlanEdit::RemoveTask(1), &[], at(3)).unwrap();
        assert_eq!(titles(&draft), vec!["One", "Three"]);
    }

    #[test]
    fn deleting_every_task_leaves_a_valid_ready_draft_with_an_empty_backlog() {
        let mut draft = ready_with(vec![task("One"), task("Two")]);
        draft.apply(PlanEdit::RemoveTask(0), &[], at(3)).unwrap();
        draft.apply(PlanEdit::RemoveTask(0), &[], at(4)).unwrap();

        assert_eq!(draft.state(), DraftState::Ready);
        assert!(draft.tasks().is_empty());
        assert_eq!(draft.spec(), "The spec.", "the spec is untouched");
        assert_eq!(draft.stories().len(), 1, "so are the stories");
    }

    #[test]
    fn an_empty_backlog_survives_being_loaded_back() {
        let mut draft = ready_with(vec![task("One")]);
        draft.apply(PlanEdit::RemoveTask(0), &[], at(3)).unwrap();
        let parts = PlanDraftParts {
            id: draft.id(),
            project_id: draft.project_id(),
            prompt: draft.prompt().to_string(),
            state: draft.state(),
            response: String::new(),
            recovered: None,
            failure: None,
            spec: draft.spec().to_string(),
            spec_edited: false,
            published_at: None,
            stories: draft.stories().to_vec(),
            tasks: Vec::new(),
            created_at: draft.created_at(),
            updated_at: draft.updated_at(),
        };
        assert_eq!(PlanDraft::from_parts(parts), Ok(draft));
    }

    #[test]
    fn tasks_can_be_reordered_and_the_order_is_kept() {
        let mut draft = ready_with(vec![task("One"), task("Two"), task("Three")]);
        draft
            .apply(PlanEdit::MoveTask { from: 2, to: 0 }, &[], at(3))
            .unwrap();
        assert_eq!(titles(&draft), vec!["Three", "One", "Two"]);
        draft
            .apply(PlanEdit::MoveTask { from: 0, to: 2 }, &[], at(4))
            .unwrap();
        assert_eq!(titles(&draft), vec!["One", "Two", "Three"]);
    }

    #[test]
    fn a_move_to_a_position_that_doesnt_exist_is_refused() {
        let mut draft = ready_with(vec![task("One")]);
        assert!(matches!(
            draft.apply(PlanEdit::MoveTask { from: 0, to: 5 }, &[], at(3)),
            Err(PlanDraftError::IndexOutOfRange { index: 5, len: 1 })
        ));
    }

    #[test]
    fn adding_a_task_appends_it_with_the_defaults() {
        let mut draft = ready_with(Vec::new());
        draft
            .apply(
                PlanEdit::AddTask {
                    title: "New work".to_string(),
                },
                &[],
                at(3),
            )
            .unwrap();
        let added = &draft.tasks()[0];
        assert_eq!(added.title, "New work");
        assert_eq!(added.priority, Priority::Medium);
        assert_eq!(added.estimate, None);
        assert_eq!(added.assignee, None);
    }

    #[test]
    fn removing_a_story_unlinks_its_tasks_and_renumbers_the_rest() {
        let mut draft = generating();
        let first = PlanTask {
            section: Some(0),
            ..task("Under first")
        };
        let second = PlanTask {
            section: Some(1),
            ..task("Under second")
        };
        draft
            .complete(
                "Spec",
                vec![story("First"), story("Second")],
                vec![first, second],
                at(2),
            )
            .unwrap();

        draft.apply(PlanEdit::RemoveStory(0), &[], at(3)).unwrap();

        assert_eq!(draft.tasks()[0].section, None);
        assert_eq!(draft.tasks()[1].section, Some(0));
    }

    #[test]
    fn edits_are_refused_for_a_draft_that_isnt_ready() {
        let mut draft = generating();
        assert_eq!(
            draft.apply(
                PlanEdit::AddTask {
                    title: "x".to_string()
                },
                &[],
                at(2)
            ),
            Err(PlanDraftError::IllegalTransition {
                state: DraftState::Generating,
                action: "be edited"
            })
        );
    }

    #[test]
    fn a_blank_spec_edit_is_refused() {
        let mut draft = ready_with(Vec::new());
        assert_eq!(
            draft.apply(PlanEdit::SetSpec("   ".to_string()), &[], at(3)),
            Err(PlanDraftError::EmptySpec)
        );
        assert_eq!(draft.spec(), "The spec.");
    }

    #[test]
    fn a_story_body_can_be_edited_and_keeps_its_line_breaks() {
        let mut draft = ready_with(Vec::new());
        draft
            .apply(
                PlanEdit::SetStoryBody {
                    index: 0,
                    body: "Line one\nLine two".to_string(),
                },
                &[],
                at(3),
            )
            .unwrap();
        assert_eq!(draft.stories()[0].body, "Line one\nLine two");
    }

    fn refined_task(title: &str, minutes: u32) -> PlanTask {
        PlanTask {
            estimate: Some(Estimate::from_minutes(i64::from(minutes)).unwrap()),
            ..task(title)
        }
    }

    #[test]
    fn refining_one_task_leaves_the_spec_stories_and_other_tasks_exactly_as_they_were() {
        let mut draft = ready_with(vec![task("One"), task("Two"), task("Three")]);
        let before = draft.clone();

        draft
            .refine(
                RefineTarget::Task(1),
                RefinedPart::Task(refined_task("Two", 30)),
                &[],
                false,
                at(3),
            )
            .unwrap();

        assert_eq!(draft.spec(), before.spec());
        assert_eq!(draft.stories(), before.stories());
        assert_eq!(draft.tasks()[0], before.tasks()[0]);
        assert_eq!(draft.tasks()[2], before.tasks()[2]);
        assert_eq!(draft.tasks()[1].estimate.map(|e| e.minutes()), Some(30));
    }

    #[test]
    fn refining_a_story_or_the_spec_leaves_the_rest_as_it_was() {
        let mut draft = ready_with(vec![task("One")]);
        let before = draft.clone();

        draft
            .refine(
                RefineTarget::Story(0),
                RefinedPart::Story(PlanStory {
                    title: "Scope, narrower".to_string(),
                    body: "Less.".to_string(),
                    edited: false,
                }),
                &[],
                false,
                at(3),
            )
            .unwrap();
        assert_eq!(draft.tasks(), before.tasks());
        assert_eq!(draft.spec(), before.spec());
        assert_eq!(draft.stories()[0].title, "Scope, narrower");

        draft
            .refine(
                RefineTarget::Spec,
                RefinedPart::Spec("A smaller spec.".to_string()),
                &[],
                false,
                at(4),
            )
            .unwrap();
        assert_eq!(draft.spec(), "A smaller spec.");
        assert_eq!(draft.stories()[0].title, "Scope, narrower");
        assert_eq!(draft.tasks(), before.tasks());
    }

    #[test]
    fn a_refinement_keeps_the_task_in_its_story() {
        let mut draft = generating();
        let linked = PlanTask {
            section: Some(0),
            ..task("Linked")
        };
        draft
            .complete("Spec", vec![story("Scope")], vec![linked], at(2))
            .unwrap();

        draft
            .refine(
                RefineTarget::Task(0),
                RefinedPart::Task(refined_task("Linked", 45)),
                &[],
                false,
                at(3),
            )
            .unwrap();

        assert_eq!(draft.tasks()[0].section, Some(0));
    }

    #[test]
    fn a_task_edited_by_hand_is_not_replaced_without_confirmation() {
        let mut draft = ready_with(vec![task("One")]);
        draft
            .apply(
                PlanEdit::SetTaskPriority {
                    index: 0,
                    priority: Priority::High,
                },
                &[],
                at(3),
            )
            .unwrap();
        let before = draft.clone();

        let refused = draft.refine(
            RefineTarget::Task(0),
            RefinedPart::Task(refined_task("One", 30)),
            &[],
            false,
            at(4),
        );

        assert_eq!(refused, Err(PlanDraftError::ManuallyEdited));
        assert_eq!(draft, before, "a refused refinement changes nothing");
    }

    #[test]
    fn a_confirmed_refinement_replaces_an_edited_task_and_clears_the_mark() {
        let mut draft = ready_with(vec![task("One")]);
        draft
            .apply(
                PlanEdit::SetTaskTitle {
                    index: 0,
                    title: "Uno".to_string(),
                },
                &[],
                at(3),
            )
            .unwrap();
        assert!(draft.tasks()[0].edited);

        draft
            .refine(
                RefineTarget::Task(0),
                RefinedPart::Task(refined_task("Uno", 30)),
                &[],
                true,
                at(4),
            )
            .unwrap();

        assert!(!draft.tasks()[0].edited);
        assert_eq!(draft.tasks()[0].estimate.map(|e| e.minutes()), Some(30));
    }

    #[test]
    fn a_generated_task_is_refined_without_asking() {
        let mut draft = ready_with(vec![task("One")]);
        assert!(!draft.tasks()[0].edited);
        assert!(
            draft
                .refine(
                    RefineTarget::Task(0),
                    RefinedPart::Task(refined_task("One", 30)),
                    &[],
                    false,
                    at(3),
                )
                .is_ok()
        );
    }

    #[test]
    fn a_spec_edited_by_hand_asks_before_it_is_refined() {
        let mut draft = ready_with(Vec::new());
        draft
            .apply(PlanEdit::SetSpec("Mine.".to_string()), &[], at(3))
            .unwrap();
        assert!(draft.spec_edited());

        assert_eq!(
            draft.refine(
                RefineTarget::Spec,
                RefinedPart::Spec("Theirs.".to_string()),
                &[],
                false,
                at(4),
            ),
            Err(PlanDraftError::ManuallyEdited)
        );
        draft
            .refine(
                RefineTarget::Spec,
                RefinedPart::Spec("Theirs.".to_string()),
                &[],
                true,
                at(5),
            )
            .unwrap();
        assert!(!draft.spec_edited());
    }

    #[test]
    fn hand_edits_mark_only_what_was_changed() {
        let mut draft = ready_with(vec![task("One"), task("Two")]);
        draft
            .apply(
                PlanEdit::SetStoryBody {
                    index: 0,
                    body: "Edited.".to_string(),
                },
                &[],
                at(3),
            )
            .unwrap();
        assert!(draft.stories()[0].edited);
        assert!(!draft.tasks()[0].edited);

        draft
            .apply(PlanEdit::MoveTask { from: 1, to: 0 }, &[], at(4))
            .unwrap();
        assert!(
            draft.tasks().iter().all(|task| !task.edited),
            "moving a task is not an edit to it"
        );

        draft
            .apply(
                PlanEdit::AddTask {
                    title: "Mine".to_string(),
                },
                &[],
                at(5),
            )
            .unwrap();
        assert!(
            draft
                .tasks()
                .iter()
                .any(|task| task.title == "Mine" && task.edited)
        );
    }

    #[test]
    fn a_refinement_for_the_wrong_part_is_refused() {
        let mut draft = ready_with(vec![task("One")]);
        assert_eq!(
            draft.refine(
                RefineTarget::Spec,
                RefinedPart::Task(refined_task("One", 30)),
                &[],
                false,
                at(3),
            ),
            Err(PlanDraftError::WrongPart)
        );
    }

    #[test]
    fn a_refinement_that_repeats_another_title_is_refused_and_changes_nothing() {
        let mut draft = ready_with(vec![task("One"), task("Two")]);
        let before = draft.clone();

        let refused = draft.refine(
            RefineTarget::Task(1),
            RefinedPart::Task(refined_task("one", 30)),
            &[],
            false,
            at(3),
        );

        assert_eq!(
            refused,
            Err(PlanDraftError::DuplicateTitle { index: 1, first: 0 })
        );
        assert_eq!(draft, before);
    }

    #[test]
    fn the_publication_plan_creates_every_task_whose_title_is_new_and_skips_the_rest() {
        let draft = ready_with(vec![task("One"), task("Two"), task("Three")]);

        let plan = draft
            .publication_plan(&["  TWO ".to_string(), "Unrelated".to_string()])
            .unwrap();

        assert_eq!(
            plan.create
                .iter()
                .map(|t| t.title.as_str())
                .collect::<Vec<_>>(),
            vec!["One", "Three"]
        );
        assert_eq!(plan.skipped, vec!["Two"]);
        assert_eq!(
            plan.create.len() + plan.skipped.len(),
            draft.tasks().len(),
            "every task of the draft is either created or skipped"
        );
    }

    #[test]
    fn a_plan_is_marked_published_once_and_then_cannot_change() {
        let mut draft = ready_with(vec![task("One")]);
        draft.mark_published(at(9)).unwrap();

        assert_eq!(draft.published_at(), Some(at(9)));
        assert_eq!(
            draft.mark_published(at(10)),
            Err(PlanDraftError::AlreadyPublished)
        );
        assert_eq!(
            draft.apply(
                PlanEdit::AddTask {
                    title: "Late".to_string()
                },
                &[],
                at(11)
            ),
            Err(PlanDraftError::AlreadyPublished)
        );
        assert_eq!(
            draft.refine(
                RefineTarget::Task(0),
                RefinedPart::Task(task("One")),
                &[],
                true,
                at(12)
            ),
            Err(PlanDraftError::AlreadyPublished)
        );
        assert_eq!(draft.tasks().len(), 1, "nothing changed after publishing");
    }

    #[test]
    fn a_draft_that_is_not_ready_cannot_be_published() {
        let mut draft = generating();
        assert!(matches!(
            draft.mark_published(at(3)),
            Err(PlanDraftError::IllegalTransition { .. })
        ));
        assert!(matches!(
            draft.publication_plan(&[]),
            Err(PlanDraftError::IllegalTransition { .. })
        ));
    }

    #[test]
    fn a_title_is_the_first_line_of_the_spec_without_heading_marks_or_the_goal_before_there_is_one()
    {
        let draft = generating();
        assert_eq!(draft.title(), "Plan the launch");

        let mut ready = generating();
        ready
            .complete("\n# Launch plan\n\nDetails.", Vec::new(), Vec::new(), at(2))
            .unwrap();
        assert_eq!(ready.title(), "Launch plan");
    }

    #[test]
    fn duplicating_a_ready_plan_copies_its_content_as_an_unpublished_plan_of_its_own() {
        let mut draft = ready_with(vec![task("One")]);
        draft.mark_published(at(5)).unwrap();

        let copy = draft
            .duplicate(crate::ids::PlanDraftId::new(), at(6))
            .unwrap();

        assert_ne!(copy.id(), draft.id());
        assert_eq!(copy.state(), DraftState::Ready);
        assert_eq!(copy.published_at(), None, "the copy is not published");
        assert_eq!(copy.spec(), draft.spec());
        assert_eq!(copy.tasks(), draft.tasks());
        assert_eq!(copy.prompt(), draft.prompt());
    }

    #[test]
    fn duplicating_an_unfinished_draft_gives_an_interrupted_copy_of_its_goal() {
        let mut draft = generating();
        draft.append_response("partial", at(2)).unwrap();

        let copy = draft
            .duplicate(crate::ids::PlanDraftId::new(), at(3))
            .unwrap();

        assert_eq!(copy.state(), DraftState::Interrupted);
        assert_eq!(copy.prompt(), draft.prompt());
        assert_eq!(copy.response(), "", "the copy starts with no text");
    }

    #[test]
    fn a_published_plan_cannot_be_resumed() {
        let mut draft = ready_with(vec![task("One")]);
        draft.mark_published(at(5)).unwrap();
        assert!(matches!(
            draft.resume(at(6)),
            Err(PlanDraftError::IllegalTransition { .. })
        ));
    }

    #[test]
    fn states_round_trip_through_their_stored_names() {
        for state in DraftState::ALL {
            assert_eq!(state.as_str().parse::<DraftState>(), Ok(state));
        }
        assert!("paused".parse::<DraftState>().is_err());
    }
}
