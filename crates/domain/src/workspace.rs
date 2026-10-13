use crate::{error::DomainError, ids::WorkspaceId, time::Timestamp, validation::trimmed_text};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workspace {
    id: WorkspaceId,
    name: String,
    icon_file: Option<String>,
    created_at: Timestamp,
    updated_at: Timestamp,
}

impl Workspace {
    pub fn new(name: &str) -> Result<Self, DomainError> {
        let now = Timestamp::now();
        Ok(Self {
            id: WorkspaceId::new(),
            name: trimmed_text(name, DomainError::EmptyName)?,
            icon_file: None,
            created_at: now,
            updated_at: now,
        })
    }

    pub fn id(&self) -> WorkspaceId {
        self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// The icon's file name inside the icons directory, if one is set.
    pub fn icon_file(&self) -> Option<&str> {
        self.icon_file.as_deref()
    }

    /// Only the file name is stored, never a path. Leaves the workspace unchanged if it's invalid.
    pub fn set_icon_file(&mut self, file_name: Option<String>) -> Result<(), DomainError> {
        if let Some(name) = &file_name {
            validate_icon_file(name)?;
        }
        self.icon_file = file_name;
        self.updated_at = Timestamp::now();
        Ok(())
    }

    pub fn created_at(&self) -> Timestamp {
        self.created_at
    }

    pub fn updated_at(&self) -> Timestamp {
        self.updated_at
    }

    /// Leaves the workspace unchanged if `name` is invalid.
    pub fn rename(&mut self, name: &str) -> Result<(), DomainError> {
        self.name = trimmed_text(name, DomainError::EmptyName)?;
        self.updated_at = Timestamp::now();
        Ok(())
    }
}

/// Stored values for a workspace. Converting it back into a [`Workspace`] re-runs validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceRecord {
    pub id: WorkspaceId,
    pub name: String,
    pub icon_file: Option<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// A plain file name: non-empty, no path separators, and not `.` or `..`.
fn validate_icon_file(file_name: &str) -> Result<(), DomainError> {
    let is_plain = !file_name.is_empty()
        && file_name != "."
        && file_name != ".."
        && !file_name.contains(['/', '\\', ':'])
        && !file_name.chars().any(char::is_control);
    if is_plain {
        Ok(())
    } else {
        Err(DomainError::InvalidIconFile {
            file_name: file_name.to_string(),
        })
    }
}

impl TryFrom<WorkspaceRecord> for Workspace {
    type Error = DomainError;

    fn try_from(record: WorkspaceRecord) -> Result<Self, Self::Error> {
        if let Some(file_name) = &record.icon_file {
            validate_icon_file(file_name)?;
        }
        Ok(Self {
            id: record.id,
            icon_file: record.icon_file,
            name: trimmed_text(&record.name, DomainError::EmptyName)?,
            created_at: record.created_at,
            updated_at: record.updated_at,
        })
    }
}

/// Checks a proposed workspace name and returns the cleaned version.
///
/// The name is trimmed, must be non-empty, and must not match another workspace
/// ignoring case. `editing` is the workspace being renamed, which may keep its own name.
pub fn validate_name<'a>(
    name: &str,
    workspaces: impl IntoIterator<Item = &'a Workspace>,
    editing: Option<WorkspaceId>,
) -> Result<String, DomainError> {
    let cleaned = trimmed_text(name, DomainError::EmptyName)?;
    let wanted = cleaned.to_lowercase();
    let taken = workspaces
        .into_iter()
        .filter(|workspace| Some(workspace.id()) != editing)
        .any(|workspace| workspace.name().to_lowercase() == wanted);
    if taken {
        return Err(DomainError::DuplicateName { name: cleaned });
    }
    Ok(cleaned)
}

/// The name for a new workspace: "Untitled workspace", then "Untitled workspace 2", and so on.
pub fn next_untitled_name<'a>(workspaces: impl IntoIterator<Item = &'a Workspace>) -> String {
    const BASE: &str = "Untitled workspace";
    let taken: Vec<String> = workspaces
        .into_iter()
        .map(|workspace| workspace.name().to_lowercase())
        .collect();
    let is_free = |candidate: &str| !taken.contains(&candidate.to_lowercase());

    if is_free(BASE) {
        return BASE.to_string();
    }
    (2..)
        .map(|number| format!("{BASE} {number}"))
        .find(|candidate| is_free(candidate))
        .expect("an unbounded sequence always yields a free name")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(name: &str) -> Workspace {
        Workspace::new(name).unwrap()
    }

    #[test]
    fn rejects_empty_and_whitespace_names() {
        let others = [named("Work")];
        assert_eq!(
            validate_name("", &others, None),
            Err(DomainError::EmptyName)
        );
        assert_eq!(
            validate_name("  \t ", &others, None),
            Err(DomainError::EmptyName)
        );
    }

    #[test]
    fn rejects_a_duplicate_name() {
        let others = [named("Work")];
        assert_eq!(
            validate_name("Work", &others, None),
            Err(DomainError::DuplicateName {
                name: "Work".to_string()
            })
        );
    }

    #[test]
    fn rejects_a_duplicate_that_differs_only_in_case_or_padding() {
        let others = [named("Work")];
        assert!(validate_name("  WORK ", &others, None).is_err());
    }

    #[test]
    fn allows_renaming_to_its_own_current_name() {
        let work = named("Work");
        let others = [work.clone()];
        assert_eq!(
            validate_name("Work", &others, Some(work.id())),
            Ok("Work".to_string())
        );
    }

    #[test]
    fn accepts_and_trims_a_valid_name() {
        let others = [named("Work")];
        assert_eq!(
            validate_name("  Personal  ", &others, None),
            Ok("Personal".to_string())
        );
    }

    #[test]
    fn untitled_names_skip_taken_numbers() {
        let existing = [named("Untitled workspace"), named("untitled workspace 2")];
        assert_eq!(next_untitled_name(&existing), "Untitled workspace 3");
    }

    #[test]
    fn first_untitled_name_is_the_plain_base() {
        assert_eq!(
            next_untitled_name(&[] as &[Workspace]),
            "Untitled workspace"
        );
    }

    #[test]
    fn rejects_blank_names() {
        assert_eq!(Workspace::new("   ").unwrap_err(), DomainError::EmptyName);
    }

    #[test]
    fn trims_names() {
        assert_eq!(Workspace::new("  Personal ").unwrap().name(), "Personal");
    }

    #[test]
    fn icon_file_must_be_a_plain_file_name() {
        let mut workspace = Workspace::new("Personal").unwrap();
        for bad in [
            "",
            ".",
            "..",
            "icons/x.png",
            "..\\x.png",
            "C:x.png",
            "bad\nname.png",
        ] {
            assert!(
                workspace.set_icon_file(Some(bad.to_string())).is_err(),
                "accepted {bad:?}"
            );
        }
        assert_eq!(workspace.icon_file(), None);
    }

    #[test]
    fn a_valid_icon_file_name_is_kept_and_can_be_cleared() {
        let mut workspace = Workspace::new("Personal").unwrap();
        workspace
            .set_icon_file(Some("workspace-1-abc.png".to_string()))
            .unwrap();
        assert_eq!(workspace.icon_file(), Some("workspace-1-abc.png"));
        workspace.set_icon_file(None).unwrap();
        assert_eq!(workspace.icon_file(), None);
    }

    #[test]
    fn failed_rename_keeps_the_old_name() {
        let mut workspace = Workspace::new("Personal").unwrap();
        assert!(workspace.rename("").is_err());
        assert_eq!(workspace.name(), "Personal");
    }
}
