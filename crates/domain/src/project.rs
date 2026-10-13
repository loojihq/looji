use crate::{
    error::DomainError,
    ids::{ProjectId, WorkspaceId},
    time::Timestamp,
    validation::trimmed_text,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    id: ProjectId,
    workspace_id: WorkspaceId,
    name: String,
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
}

/// Stored values for a project. Converting it back into a [`Project`] re-runs validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectRecord {
    pub id: ProjectId,
    pub workspace_id: WorkspaceId,
    pub name: String,
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
            created_at: record.created_at,
            updated_at: record.updated_at,
        })
    }
}
