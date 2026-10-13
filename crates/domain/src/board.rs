//! Board columns. A column is how one project's board shows a status: its
//! label, its place in the row, and whether it's visible.

use crate::{
    error::DomainError,
    ids::{ColumnId, ProjectId},
    position::Position,
    status::Status,
    validation::trimmed_text,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Column {
    id: ColumnId,
    project_id: ProjectId,
    status: Status,
    name: String,
    position: Position,
    visible: bool,
}

impl Column {
    pub fn new(
        project_id: ProjectId,
        status: Status,
        name: &str,
        position: Position,
        visible: bool,
    ) -> Result<Self, DomainError> {
        Ok(Self {
            id: ColumnId::new(),
            project_id,
            status,
            name: trimmed_text(name, DomainError::EmptyName)?,
            position,
            visible,
        })
    }

    pub fn id(&self) -> ColumnId {
        self.id
    }

    pub fn project_id(&self) -> ProjectId {
        self.project_id
    }

    pub fn status(&self) -> Status {
        self.status
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn position(&self) -> &Position {
        &self.position
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    /// Leaves the column unchanged if `name` is invalid.
    pub fn rename(&mut self, name: &str) -> Result<(), DomainError> {
        self.name = trimmed_text(name, DomainError::EmptyName)?;
        Ok(())
    }

    pub fn set_position(&mut self, position: Position) {
        self.position = position;
    }

    pub fn set_visible(&mut self, visible: bool) {
        self.visible = visible;
    }
}

/// Stored values for a column. Converting it back into a [`Column`] re-runs validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnRecord {
    pub id: ColumnId,
    pub project_id: ProjectId,
    pub status: Status,
    pub name: String,
    pub position: Position,
    pub visible: bool,
}

impl TryFrom<ColumnRecord> for Column {
    type Error = DomainError;

    fn try_from(record: ColumnRecord) -> Result<Self, Self::Error> {
        Ok(Self {
            id: record.id,
            project_id: record.project_id,
            status: record.status,
            name: trimmed_text(&record.name, DomainError::EmptyName)?,
            position: record.position,
            visible: record.visible,
        })
    }
}
