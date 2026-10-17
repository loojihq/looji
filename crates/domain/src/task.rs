use crate::{
    error::DomainError,
    ids::{ProjectId, TaskId, TeamMemberId},
    position::Position,
    status::Status,
    task_fields::{DueDate, MAX_TAGS, Priority, TagError, normalize_tag},
    time::Timestamp,
    validation::trimmed_text,
};

/// Estimated effort, in whole minutes. Minutes avoid float drift in working-time calculations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Estimate(u32);

impl Estimate {
    pub fn from_minutes(minutes: i64) -> Result<Self, DomainError> {
        if minutes < 0 {
            return Err(DomainError::NegativeEstimate { minutes });
        }
        u32::try_from(minutes)
            .map(Self)
            .map_err(|_| DomainError::EstimateTooLarge)
    }

    /// Converts hours to whole minutes, rounding to the nearest minute.
    pub fn from_hours(hours: f64) -> Result<Self, DomainError> {
        if !hours.is_finite() {
            return Err(DomainError::NotFinite);
        }
        let minutes = (hours * 60.0).round();
        if minutes < 0.0 {
            return Err(DomainError::NegativeEstimate {
                minutes: minutes as i64,
            });
        }
        if minutes > f64::from(u32::MAX) {
            return Err(DomainError::EstimateTooLarge);
        }
        Ok(Self(minutes as u32))
    }

    pub fn minutes(&self) -> u32 {
        self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Task {
    id: TaskId,
    project_id: ProjectId,
    title: String,
    description: String,
    status: Status,
    position: Position,
    estimate: Option<Estimate>,
    assignee_id: Option<TeamMemberId>,
    priority: Option<Priority>,
    due_on: Option<DueDate>,
    /// When the work starts. A task with a start and a due date can be drawn on the roadmap.
    start_on: Option<DueDate>,
    tags: Vec<String>,
    created_at: Timestamp,
    updated_at: Timestamp,
}

impl Task {
    pub fn new(
        project_id: ProjectId,
        title: &str,
        status: Status,
        position: Position,
    ) -> Result<Self, DomainError> {
        let now = Timestamp::now();
        Ok(Self {
            id: TaskId::new(),
            project_id,
            title: trimmed_text(title, DomainError::EmptyTitle)?,
            description: String::new(),
            status,
            position,
            estimate: None,
            assignee_id: None,
            priority: None,
            due_on: None,
            start_on: None,
            tags: Vec::new(),
            created_at: now,
            updated_at: now,
        })
    }

    pub fn id(&self) -> TaskId {
        self.id
    }

    pub fn project_id(&self) -> ProjectId {
        self.project_id
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn description(&self) -> &str {
        &self.description
    }

    pub fn status(&self) -> Status {
        self.status
    }

    pub fn position(&self) -> &Position {
        &self.position
    }

    pub fn estimate(&self) -> Option<Estimate> {
        self.estimate
    }

    pub fn priority(&self) -> Option<Priority> {
        self.priority
    }

    pub fn due_on(&self) -> Option<DueDate> {
        self.due_on
    }

    pub fn start_on(&self) -> Option<DueDate> {
        self.start_on
    }

    /// The tags, sorted and without duplicates.
    pub fn tags(&self) -> &[String] {
        &self.tags
    }

    pub fn assignee_id(&self) -> Option<TeamMemberId> {
        self.assignee_id
    }

    pub fn created_at(&self) -> Timestamp {
        self.created_at
    }

    pub fn updated_at(&self) -> Timestamp {
        self.updated_at
    }

    /// Leaves the task unchanged if `title` is invalid.
    pub fn rename(&mut self, title: &str) -> Result<(), DomainError> {
        self.title = trimmed_text(title, DomainError::EmptyTitle)?;
        self.touch();
        Ok(())
    }

    pub fn set_description(&mut self, description: impl Into<String>) {
        self.description = description.into();
        self.touch();
    }

    pub fn set_status(&mut self, status: Status) {
        self.status = status;
        self.touch();
    }

    pub fn set_position(&mut self, position: Position) {
        self.position = position;
        self.touch();
    }

    /// `None` clears the estimate. Leaves the task unchanged if the minutes are invalid.
    pub fn set_estimate_minutes(&mut self, minutes: Option<i64>) -> Result<(), DomainError> {
        self.estimate = minutes.map(Estimate::from_minutes).transpose()?;
        self.touch();
        Ok(())
    }

    pub fn set_priority(&mut self, priority: Option<Priority>) {
        self.priority = priority;
        self.touch();
    }

    pub fn set_due_on(&mut self, due_on: Option<DueDate>) {
        self.due_on = due_on;
        self.touch();
    }

    /// The start is stored as given. A start after the due date is allowed, and the roadmap flags it, so bad
    /// data stays visible rather than being swapped.
    pub fn set_start_on(&mut self, start_on: Option<DueDate>) {
        self.start_on = start_on;
        self.touch();
    }

    /// Replaces the tags. They are sorted and de-duplicated, so the order given doesn't matter.
    pub fn set_tags(&mut self, mut tags: Vec<String>) {
        tags.sort();
        tags.dedup();
        self.tags = tags;
        self.touch();
    }

    /// Adds one tag in stored form. A tag the task already has changes nothing, so "Web" after "web"
    /// is not a change. Leaves the task unchanged if the tag is invalid or the task is full.
    pub fn add_tag(&mut self, tag: &str) -> Result<(), DomainError> {
        let tag = normalize_tag(tag).map_err(DomainError::Tag)?;
        if self.tags.contains(&tag) {
            return Ok(());
        }
        if self.tags.len() >= MAX_TAGS {
            return Err(DomainError::Tag(TagError::TooMany { max: MAX_TAGS }));
        }
        self.tags.push(tag);
        self.tags.sort();
        self.touch();
        Ok(())
    }

    /// Removes one tag by its stored form. A tag the task doesn't have changes nothing.
    pub fn remove_tag(&mut self, tag: &str) {
        let before = self.tags.len();
        self.tags.retain(|existing| existing != tag);
        if self.tags.len() != before {
            self.touch();
        }
    }

    /// `None` clears the estimate.
    pub fn set_estimate(&mut self, estimate: Option<Estimate>) {
        self.estimate = estimate;
        self.touch();
    }

    pub fn assign(&mut self, assignee: Option<TeamMemberId>) {
        self.assignee_id = assignee;
        self.touch();
    }

    fn touch(&mut self) {
        self.updated_at = Timestamp::now();
    }
}

/// Stored values for a task. Converting it back into a [`Task`] re-runs validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskRecord {
    pub id: TaskId,
    pub project_id: ProjectId,
    pub title: String,
    pub description: String,
    pub status: Status,
    pub position: Position,
    pub estimate_minutes: Option<i64>,
    pub assignee_id: Option<TeamMemberId>,
    pub priority: Option<Priority>,
    pub due_on: Option<DueDate>,
    pub start_on: Option<DueDate>,
    pub tags: Vec<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl TryFrom<TaskRecord> for Task {
    type Error = DomainError;

    fn try_from(record: TaskRecord) -> Result<Self, Self::Error> {
        Ok(Self {
            id: record.id,
            project_id: record.project_id,
            title: trimmed_text(&record.title, DomainError::EmptyTitle)?,
            description: record.description,
            status: record.status,
            position: record.position,
            estimate: record
                .estimate_minutes
                .map(Estimate::from_minutes)
                .transpose()?,
            assignee_id: record.assignee_id,
            priority: record.priority,
            due_on: record.due_on,
            start_on: record.start_on,
            tags: record.tags,
            created_at: record.created_at,
            updated_at: record.updated_at,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task_titled(title: &str) -> Result<Task, DomainError> {
        Task::new(
            ProjectId::new(),
            title,
            Status::Todo,
            Position::new("a0").unwrap(),
        )
    }

    fn sample_task() -> Task {
        task_titled("Write the board").unwrap()
    }

    #[test]
    fn rejects_empty_title() {
        assert_eq!(task_titled("").unwrap_err(), DomainError::EmptyTitle);
    }

    #[test]
    fn rejects_whitespace_only_title() {
        assert_eq!(task_titled(" \t\n ").unwrap_err(), DomainError::EmptyTitle);
    }

    #[test]
    fn trims_title() {
        assert_eq!(task_titled("  Ship it  ").unwrap().title(), "Ship it");
    }

    #[test]
    fn rejects_title_over_the_limit() {
        let long = "x".repeat(201);
        assert_eq!(
            task_titled(&long).unwrap_err(),
            DomainError::TooLong { max: 200 }
        );
        assert!(task_titled(&"x".repeat(200)).is_ok());
    }

    #[test]
    fn rename_rejects_blank_and_keeps_the_old_title() {
        let mut task = sample_task();
        assert_eq!(task.rename("   ").unwrap_err(), DomainError::EmptyTitle);
        assert_eq!(task.title(), "Write the board");
    }

    #[test]
    fn added_tags_are_normalised_and_a_repeat_changes_nothing() {
        let mut task = sample_task();
        task.add_tag("  Web  Design ").unwrap();
        task.add_tag("WEB design").unwrap();
        assert_eq!(task.tags(), ["web design"]);
    }

    #[test]
    fn a_blank_or_comma_tag_is_refused_and_leaves_the_task_alone() {
        let mut task = sample_task();
        assert!(task.add_tag("   ").is_err());
        assert!(task.add_tag("a,b").is_err());
        assert!(task.tags().is_empty());
    }

    #[test]
    fn a_task_full_of_tags_refuses_another() {
        let mut task = sample_task();
        for n in 0..crate::task_fields::MAX_TAGS {
            task.add_tag(&format!("tag{n}")).unwrap();
        }
        assert!(task.add_tag("one more").is_err());
        assert_eq!(task.tags().len(), crate::task_fields::MAX_TAGS);
    }

    #[test]
    fn removing_a_tag_drops_only_that_tag() {
        let mut task = sample_task();
        task.add_tag("alpha").unwrap();
        task.add_tag("beta").unwrap();
        task.remove_tag("alpha");
        task.remove_tag("missing");
        assert_eq!(task.tags(), ["beta"]);
    }

    #[test]
    fn zero_estimate_is_allowed() {
        let mut task = sample_task();
        task.set_estimate_minutes(Some(0)).unwrap();
        assert_eq!(task.estimate().map(|e| e.minutes()), Some(0));
    }

    #[test]
    fn negative_estimate_is_rejected() {
        let mut task = sample_task();
        assert_eq!(
            task.set_estimate_minutes(Some(-5)).unwrap_err(),
            DomainError::NegativeEstimate { minutes: -5 }
        );
        assert_eq!(task.estimate(), None);
    }

    #[test]
    fn failed_estimate_keeps_the_previous_value() {
        let mut task = sample_task();
        task.set_estimate_minutes(Some(30)).unwrap();
        assert!(task.set_estimate_minutes(Some(-1)).is_err());
        assert_eq!(task.estimate().map(|e| e.minutes()), Some(30));
    }

    #[test]
    fn nan_hours_are_rejected() {
        assert_eq!(
            Estimate::from_hours(f64::NAN).unwrap_err(),
            DomainError::NotFinite
        );
    }

    #[test]
    fn infinite_hours_are_rejected() {
        assert_eq!(
            Estimate::from_hours(f64::INFINITY).unwrap_err(),
            DomainError::NotFinite
        );
    }

    #[test]
    fn hours_round_to_whole_minutes() {
        assert_eq!(Estimate::from_hours(1.5).unwrap().minutes(), 90);
        assert_eq!(Estimate::from_hours(0.0).unwrap().minutes(), 0);
    }

    #[test]
    fn stored_record_with_blank_title_fails_to_load() {
        let record = TaskRecord {
            id: TaskId::new(),
            project_id: ProjectId::new(),
            title: "   ".to_string(),
            description: String::new(),
            status: Status::Todo,
            position: Position::new("a0").unwrap(),
            estimate_minutes: None,
            assignee_id: None,
            priority: None,
            due_on: None,
            start_on: None,
            tags: Vec::new(),
            created_at: Timestamp::from_unix_millis(0),
            updated_at: Timestamp::from_unix_millis(0),
        };
        assert_eq!(Task::try_from(record).unwrap_err(), DomainError::EmptyTitle);
    }
}
