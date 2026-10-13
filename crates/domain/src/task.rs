use crate::{
    error::DomainError,
    ids::{ProjectId, TaskId, TeamMemberId},
    position::Position,
    status::Status,
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
            created_at: Timestamp::from_unix_millis(0),
            updated_at: Timestamp::from_unix_millis(0),
        };
        assert_eq!(Task::try_from(record).unwrap_err(), DomainError::EmptyTitle);
    }
}
