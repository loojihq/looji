//! Reads a planning response. The response is untrusted input, so it is read in two steps, and each step's failures
//! are reported with the reason.
//!
//! 1. Structure. The text must be one JSON object with exactly the fields the schema names, in the types it names.
//!    Unknown fields and missing fields are errors. A cut-off response is reported as `Truncated`, and a response
//!    wrapped in prose or a code fence is reported as such, so the caller knows what to ask for again.
//! 2. Rules. Every rule is checked and every failure is collected, so one reply shows all its problems at once.
//!    The rules are the domain's: titles, priorities, estimates, and tags are checked by the same functions the
//!    rest of the app uses.
//!
//! Warnings and errors are decided here, and only here:
//! - An assignee that matches no team member, or that matches two or more, is a warning. The task is kept, its
//!   assignee is null, and the warning says why. Assignees aren't trusted, so an invented name can't assign a
//!   task to anyone.
//! - Everything else is an error, and the whole response is refused. That covers blank or repeated titles, unknown
//!   priorities, bad estimates, bad tags, an empty task list, and a blank spec.

use std::str::FromStr;

use looji_domain::{
    DomainError, Estimate, Priority, TeamMember, TeamMemberId, display_labels, normalize_tags,
    parse_estimate_hours, validate_member_name, validate_title,
};
use serde::Deserialize;
use serde_json::{error::Category, value::RawValue};
use thiserror::Error;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawPlan {
    spec: String,
    user_stories: Vec<RawStory>,
    tasks: Vec<RawTask>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawStory {
    title: String,
    body: String,
}

/// Estimate and assignee are read raw, and not as numbers and strings, because some forms must be told apart.
/// A number written as `1e3` or a string written as `"2 days"` is a specific error, not a schema one. A raw field is
/// still required, so a missing key is a schema error.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTask {
    title: String,
    priority: String,
    estimate_hours: Box<RawValue>,
    assignee: Box<RawValue>,
    tags: Vec<String>,
}

/// The kinds of failure, without their details. Tests compare these, so a case is judged by the kind it produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    Empty,
    CodeFence,
    ProseBeforeJson,
    Truncated,
    Malformed,
    Schema,
    Spec,
    NoTasks,
    Title,
    Priority,
    Estimate,
    Assignee,
    Tag,
    DuplicateTitle,
}

/// One broken rule, and where it is in the response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanIssue {
    /// Where the problem is, such as `tasks[2].estimate_hours`.
    pub path: String,
    pub problem: Problem,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    BlankSpec,
    NoTasks,
    /// Why the title was refused.
    Title(String),
    /// The priority as written. Only the exact names are accepted.
    Priority(String),
    /// Why the estimate was refused.
    Estimate(String),
    /// Why the assignee was refused. Only a team name or null is accepted, and a name that matches no member is a
    /// warning, not this.
    Assignee(String),
    /// Why the tag was refused.
    Tag(String),
    /// The task repeats the title of an earlier one, ignoring case. `first` is the earlier task's index.
    DuplicateTitle {
        first: usize,
    },
}

impl Problem {
    pub fn kind(&self) -> ErrorKind {
        match self {
            Problem::BlankSpec => ErrorKind::Spec,
            Problem::NoTasks => ErrorKind::NoTasks,
            Problem::Title(_) => ErrorKind::Title,
            Problem::Priority(_) => ErrorKind::Priority,
            Problem::Estimate(_) => ErrorKind::Estimate,
            Problem::Assignee(_) => ErrorKind::Assignee,
            Problem::Tag(_) => ErrorKind::Tag,
            Problem::DuplicateTitle { .. } => ErrorKind::DuplicateTitle,
        }
    }
}

impl std::fmt::Display for PlanIssue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let message = match &self.problem {
            Problem::BlankSpec => "the spec is blank".to_string(),
            Problem::NoTasks => "the plan has no tasks".to_string(),
            Problem::Title(reason)
            | Problem::Estimate(reason)
            | Problem::Assignee(reason)
            | Problem::Tag(reason) => reason.clone(),
            Problem::Priority(value) => {
                format!("\"{value}\" is not a priority; use \"low\", \"medium\", or \"high\"")
            }
            Problem::DuplicateTitle { first } => {
                format!("repeats the title of task {first}, ignoring case")
            }
        };
        write!(f, "{}: {message}", self.path)
    }
}

/// Why a response was refused. The first four are about the text itself, and the rest are about the structure
/// or the rules.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PlanParseError {
    #[error("the response is empty")]
    Empty,
    #[error("the response is wrapped in a code fence; reply with the bare JSON object")]
    CodeFence,
    #[error("the response has text before the JSON object; reply with only the object")]
    ProseBeforeJson,
    #[error(
        "the response was cut off before the JSON object ended; it may have reached the token limit"
    )]
    Truncated,
    #[error("the response isn't valid JSON: {0}")]
    Malformed(String),
    #[error("the response doesn't match the schema: {0}")]
    Schema(String),
    #[error("the plan breaks {} rule(s): {}", .0.len(), join(.0))]
    Invalid(Vec<PlanIssue>),
}

fn join(issues: &[PlanIssue]) -> String {
    issues
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ")
}

impl PlanParseError {
    /// The kinds this refusal is made of. One for most refusals, and one per broken rule for `Invalid`.
    pub fn kinds(&self) -> Vec<ErrorKind> {
        match self {
            PlanParseError::Empty => vec![ErrorKind::Empty],
            PlanParseError::CodeFence => vec![ErrorKind::CodeFence],
            PlanParseError::ProseBeforeJson => vec![ErrorKind::ProseBeforeJson],
            PlanParseError::Truncated => vec![ErrorKind::Truncated],
            PlanParseError::Malformed(_) => vec![ErrorKind::Malformed],
            PlanParseError::Schema(_) => vec![ErrorKind::Schema],
            PlanParseError::Invalid(issues) => {
                issues.iter().map(|issue| issue.problem.kind()).collect()
            }
        }
    }
}

/// A user story, as the plan has it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserStory {
    pub title: String,
    pub body: String,
}

/// A task that passed every rule. Its assignee is a team member's id, or null.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedTask {
    pub title: String,
    pub priority: Priority,
    pub estimate: Option<Estimate>,
    pub assignee: Option<TeamMemberId>,
    pub tags: Vec<String>,
}

/// Something the parser noticed that doesn't stop the plan. `task` is the task's index, from zero.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanWarning {
    /// The name matched no team member, so the task is unassigned.
    UnknownAssignee { task: usize, name: String },
    /// The name matched two or more team members, so the task is unassigned rather than guessing which one.
    AmbiguousAssignee { task: usize, name: String },
}

/// A response that passed every rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedPlan {
    pub spec: String,
    pub user_stories: Vec<UserStory>,
    pub tasks: Vec<ParsedTask>,
    pub warnings: Vec<PlanWarning>,
}

/// Parses and checks a planning response against the team it was asked to assign into.
///
/// Call this with the finished text only. A partial stream isn't a response yet.
pub fn parse_plan(text: &str, team: &[TeamMember]) -> Result<ParsedPlan, PlanParseError> {
    let text = text.trim();
    if text.is_empty() {
        return Err(PlanParseError::Empty);
    }
    if text.starts_with("```") {
        return Err(PlanParseError::CodeFence);
    }
    if !text.starts_with('{') {
        return Err(PlanParseError::ProseBeforeJson);
    }
    let raw = read_plan(text)?;

    let mut issues = Vec::new();
    if raw.spec.trim().is_empty() {
        issues.push(issue("spec", Problem::BlankSpec));
    }
    if raw.tasks.is_empty() {
        issues.push(issue("tasks", Problem::NoTasks));
    }
    let mut user_stories = Vec::new();
    for (index, story) in raw.user_stories.iter().enumerate() {
        match validate_title(&story.title) {
            Ok(title) => user_stories.push(UserStory {
                title,
                body: story.body.trim().to_string(),
            }),
            Err(err) => issues.push(issue(
                &format!("user_stories[{index}].title"),
                Problem::Title(title_reason(&err)),
            )),
        }
    }

    let labels = display_labels(team);
    let mut tasks = Vec::new();
    let mut warnings = Vec::new();
    let mut titles: Vec<(String, usize)> = Vec::new();
    for (index, task) in raw.tasks.iter().enumerate() {
        let title = match validate_title(&task.title) {
            Ok(title) => {
                let key = title.to_lowercase();
                match titles.iter().find(|(seen, _)| *seen == key) {
                    Some((_, first)) => issues.push(issue(
                        &format!("tasks[{index}].title"),
                        Problem::DuplicateTitle { first: *first },
                    )),
                    None => titles.push((key, index)),
                }
                Some(title)
            }
            Err(err) => {
                issues.push(issue(
                    &format!("tasks[{index}].title"),
                    Problem::Title(title_reason(&err)),
                ));
                None
            }
        };

        let priority = match Priority::from_str(&task.priority) {
            Ok(priority) => Some(priority),
            Err(_) => {
                issues.push(issue(
                    &format!("tasks[{index}].priority"),
                    Problem::Priority(task.priority.clone()),
                ));
                None
            }
        };

        let estimate = match read_estimate(&task.estimate_hours) {
            Ok(estimate) => Some(estimate),
            Err(reason) => {
                issues.push(issue(
                    &format!("tasks[{index}].estimate_hours"),
                    Problem::Estimate(reason),
                ));
                None
            }
        };

        let assignee = match read_assignee(&task.assignee) {
            Ok(Some(name)) => {
                let (member, warning) = resolve_assignee(&name, index, &labels, team);
                warnings.extend(warning);
                Some(member)
            }
            Ok(None) => Some(None),
            Err(reason) => {
                issues.push(issue(
                    &format!("tasks[{index}].assignee"),
                    Problem::Assignee(reason),
                ));
                None
            }
        };

        match normalize_tags(task.tags.iter().map(String::as_str)) {
            Ok(tags) => {
                if let (Some(title), Some(priority), Some(estimate), Some(assignee)) =
                    (title, priority, estimate, assignee)
                {
                    tasks.push(ParsedTask {
                        title,
                        priority,
                        estimate,
                        assignee,
                        tags,
                    });
                }
            }
            Err(err) => issues.push(issue(
                &format!("tasks[{index}].tags"),
                Problem::Tag(err.to_string()),
            )),
        }
    }

    if !issues.is_empty() {
        return Err(PlanParseError::Invalid(issues));
    }
    Ok(ParsedPlan {
        spec: raw.spec.trim().to_string(),
        user_stories,
        tasks,
        warnings,
    })
}

/// Why a title was refused, in words that fit a story and a task alike.
fn title_reason(err: &DomainError) -> String {
    match err {
        DomainError::EmptyTitle => "the title can't be empty".to_string(),
        DomainError::TooLong { max } => format!("the title can be at most {max} characters"),
        other => other.to_string(),
    }
}

fn issue(path: &str, problem: Problem) -> PlanIssue {
    PlanIssue {
        path: path.to_string(),
        problem,
    }
}

/// Reads the structure. Checks that the text is one object and nothing follows it, and turns each serde failure
/// into its kind.
pub(super) fn read_plan(text: &str) -> Result<RawPlan, PlanParseError> {
    let mut de = serde_json::Deserializer::from_str(text);
    let plan = RawPlan::deserialize(&mut de).map_err(|err| match err.classify() {
        Category::Eof => PlanParseError::Truncated,
        Category::Data => PlanParseError::Schema(err.to_string()),
        Category::Syntax | Category::Io => PlanParseError::Malformed(err.to_string()),
    })?;
    de.end()
        .map_err(|err| PlanParseError::Malformed(format!("text follows the JSON object: {err}")))?;
    Ok(plan)
}

/// An estimate in hours. A JSON number is read through the domain's rule, which takes the same text, so the two
/// can't disagree. A string is refused, since the number is what the schema asks for. So is exponent form, which
/// the domain would accept but the prompt doesn't allow.
fn read_estimate(raw: &RawValue) -> Result<Option<Estimate>, String> {
    let text = raw.get().trim();
    if text == "null" {
        return Ok(None);
    }
    if text.starts_with('"') {
        return Err(
            "the estimate must be a number of hours, not text such as \"3h\" or \"2 days\""
                .to_string(),
        );
    }
    if text.contains(['e', 'E']) {
        return Err(
            "write the estimate as plain hours, such as 2 or 2.5, not in exponent form".to_string(),
        );
    }
    parse_estimate_hours(text).map_err(|err| err.to_string())
}

/// The assignee as written, or `None` when it's null. Only a string or null is accepted.
fn read_assignee(raw: &RawValue) -> Result<Option<String>, String> {
    let text = raw.get().trim();
    if text == "null" {
        return Ok(None);
    }
    serde_json::from_str::<String>(text)
        .map(Some)
        .map_err(|_| "the assignee must be a team name or null".to_string())
}

/// Matches an assignee against the team. The labels come from `display_labels`, so a name shared by two members
/// is matched as "Name (1)" and "Name (2)". The plain name then matches nothing, because it can't say which
/// member was meant.
///
/// Returns the member, or null with a warning when the name matches none, or matches two or more without a label.
fn resolve_assignee(
    name: &str,
    task: usize,
    labels: &[String],
    team: &[TeamMember],
) -> (Option<TeamMemberId>, Option<PlanWarning>) {
    let written = name.trim();
    if written.is_empty() {
        return (None, None);
    }
    let wanted = validate_member_name(written)
        .unwrap_or_else(|_| written.to_string())
        .to_lowercase();
    if let Some(index) = labels
        .iter()
        .position(|label| label.to_lowercase() == wanted)
    {
        return (Some(team[index].id()), None);
    }
    let matches: Vec<&TeamMember> = team
        .iter()
        .filter(|member| member.name().to_lowercase() == wanted)
        .collect();
    match matches.as_slice() {
        [member] => (Some(member.id()), None),
        [] => (
            None,
            Some(PlanWarning::UnknownAssignee {
                task,
                name: written.to_string(),
            }),
        ),
        _ => (
            None,
            Some(PlanWarning::AmbiguousAssignee {
                task,
                name: written.to_string(),
            }),
        ),
    }
}
