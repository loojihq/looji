use crate::{
    error::DomainError,
    ids::{ProjectId, WorkspaceId},
    status::Status,
    status_visibility::check_hideable,
    time::Timestamp,
    validation::trimmed_text,
    working_schedule::WorkingSchedule,
};

/// Longest project description, in characters after trimming.
pub const MAX_DESCRIPTION_CHARS: usize = 500;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    id: ProjectId,
    workspace_id: WorkspaceId,
    name: String,
    description: Option<String>,
    /// Whether the In review column is shown. The defaults are always shown.
    show_in_review: bool,
    /// Whether the Backlog column is shown. The defaults are always shown.
    show_backlog: bool,
    /// When the project works. A new project gets the default: Monday to Friday, 09:00 to 17:00.
    working_schedule: WorkingSchedule,
    created_at: Timestamp,
    updated_at: Timestamp,
}

impl Project {
    pub fn new(workspace_id: WorkspaceId, name: &str) -> Result<Self, DomainError> {
        let now = Timestamp::now();
        Ok(Self {
            id: ProjectId::new(),
            workspace_id,
            name: trimmed_text(name, DomainError::EmptyName)?,
            description: None,
            show_in_review: true,
            show_backlog: true,
            working_schedule: WorkingSchedule::default(),
            created_at: now,
            updated_at: now,
        })
    }

    pub fn id(&self) -> ProjectId {
        self.id
    }

    pub fn workspace_id(&self) -> WorkspaceId {
        self.workspace_id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn show_in_review(&self) -> bool {
        self.show_in_review
    }

    pub fn show_backlog(&self) -> bool {
        self.show_backlog
    }

    pub fn working_schedule(&self) -> WorkingSchedule {
        self.working_schedule
    }

    /// Replaces the working schedule. The schedule was checked when it was built, so this can't fail.
    pub fn set_working_schedule(&mut self, schedule: WorkingSchedule) {
        self.working_schedule = schedule;
        self.updated_at = Timestamp::now();
    }

    /// Whether `status` has a column on this project's board. Defaults are always visible.
    /// The statuses this project's board shows, in board order.
    pub fn visible_statuses(&self) -> Vec<Status> {
        Status::ALL
            .into_iter()
            .filter(|status| self.is_status_visible(*status))
            .collect()
    }

    pub fn is_status_visible(&self, status: Status) -> bool {
        match status {
            Status::Backlog => self.show_backlog,
            Status::InReview => self.show_in_review,
            _ => true,
        }
    }

    /// Shows or hides an optional status column. Hiding a default fails. This only sets the
    /// flag; moving the tasks in that column is the persistence layer's job, in the same
    /// transaction.
    pub fn set_status_visible(&mut self, status: Status, visible: bool) -> Result<(), DomainError> {
        if !visible {
            check_hideable(status)?;
        }
        let flag = match status {
            Status::Backlog => &mut self.show_backlog,
            Status::InReview => &mut self.show_in_review,
            _ => return Ok(()),
        };
        *flag = visible;
        self.updated_at = Timestamp::now();
        Ok(())
    }

    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    pub fn created_at(&self) -> Timestamp {
        self.created_at
    }

    pub fn updated_at(&self) -> Timestamp {
        self.updated_at
    }

    /// Leaves the project unchanged if `name` is invalid.
    pub fn rename(&mut self, name: &str) -> Result<(), DomainError> {
        self.name = trimmed_text(name, DomainError::EmptyName)?;
        self.updated_at = Timestamp::now();
        Ok(())
    }

    /// Blank or missing text clears the description. Leaves the project unchanged if the text
    /// is too long.
    pub fn set_description(&mut self, description: Option<&str>) -> Result<(), DomainError> {
        self.description = clean_description(description)?;
        self.updated_at = Timestamp::now();
        Ok(())
    }
}

/// Checks a proposed project name against the other projects in the same workspace.
///
/// Trimmed and compared ignoring case. Pass only the projects of one workspace.
pub fn validate_project_name<'a>(
    name: &str,
    projects: impl IntoIterator<Item = &'a Project>,
    editing: Option<ProjectId>,
) -> Result<String, DomainError> {
    let cleaned = trimmed_text(name, DomainError::EmptyName)?;
    let wanted = cleaned.to_lowercase();
    let taken = projects
        .into_iter()
        .filter(|project| Some(project.id()) != editing)
        .any(|project| project.name().to_lowercase() == wanted);
    if taken {
        return Err(DomainError::DuplicateProjectName { name: cleaned });
    }
    Ok(cleaned)
}

/// Trims a description. Blank text becomes `None`. Text over the limit is an error.
pub fn clean_description(description: Option<&str>) -> Result<Option<String>, DomainError> {
    let trimmed = description.map(str::trim).filter(|text| !text.is_empty());
    match trimmed {
        Some(text) if text.chars().count() > MAX_DESCRIPTION_CHARS => {
            Err(DomainError::DescriptionTooLong {
                max: MAX_DESCRIPTION_CHARS,
            })
        }
        Some(text) => Ok(Some(text.to_string())),
        None => Ok(None),
    }
}

/// Stored values for a project. Converting it back into a [`Project`] re-runs validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectRecord {
    pub id: ProjectId,
    pub workspace_id: WorkspaceId,
    pub name: String,
    pub description: Option<String>,
    pub show_in_review: bool,
    pub show_backlog: bool,
    pub working_start_min: i64,
    pub working_end_min: i64,
    pub workdays_mask: i64,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl TryFrom<ProjectRecord> for Project {
    type Error = DomainError;

    fn try_from(record: ProjectRecord) -> Result<Self, Self::Error> {
        Ok(Self {
            id: record.id,
            workspace_id: record.workspace_id,
            name: trimmed_text(&record.name, DomainError::EmptyName)?,
            description: clean_description(record.description.as_deref())?,
            show_in_review: record.show_in_review,
            show_backlog: record.show_backlog,
            working_schedule: WorkingSchedule::from_stored(
                record.working_start_min,
                record.working_end_min,
                record.workdays_mask,
            )
            .map_err(DomainError::Schedule)?,
            created_at: record.created_at,
            updated_at: record.updated_at,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(workspace: WorkspaceId, name: &str) -> Project {
        Project::new(workspace, name).unwrap()
    }

    #[test]
    fn names_are_unique_ignoring_case_and_padding_within_a_workspace() {
        let workspace = WorkspaceId::new();
        let existing = [project(workspace, "Website")];
        assert!(matches!(
            validate_project_name("  website ", &existing, None),
            Err(DomainError::DuplicateProjectName { .. })
        ));
    }

    #[test]
    fn a_project_may_keep_its_own_name() {
        let workspace = WorkspaceId::new();
        let website = project(workspace, "Website");
        let existing = [website.clone()];
        assert_eq!(
            validate_project_name("Website", &existing, Some(website.id())),
            Ok("Website".to_string())
        );
    }

    #[test]
    fn blank_names_are_rejected() {
        assert_eq!(
            validate_project_name("   ", &[] as &[Project], None),
            Err(DomainError::EmptyName)
        );
    }

    #[test]
    fn descriptions_are_trimmed_and_blank_ones_become_none() {
        assert_eq!(
            clean_description(Some("  Launch site  ")),
            Ok(Some("Launch site".into()))
        );
        assert_eq!(clean_description(Some("   ")), Ok(None));
        assert_eq!(clean_description(None), Ok(None));
    }

    #[test]
    fn descriptions_over_the_limit_are_rejected() {
        let long = "x".repeat(MAX_DESCRIPTION_CHARS + 1);
        assert_eq!(
            clean_description(Some(&long)),
            Err(DomainError::DescriptionTooLong {
                max: MAX_DESCRIPTION_CHARS
            })
        );
        assert!(clean_description(Some(&"x".repeat(MAX_DESCRIPTION_CHARS))).is_ok());
    }

    #[test]
    fn a_failed_description_keeps_the_previous_one() {
        let mut project = project(WorkspaceId::new(), "Website");
        project.set_description(Some("Keep me")).unwrap();
        assert!(
            project
                .set_description(Some(&"x".repeat(MAX_DESCRIPTION_CHARS + 1)))
                .is_err()
        );
        assert_eq!(project.description(), Some("Keep me"));
    }
}
