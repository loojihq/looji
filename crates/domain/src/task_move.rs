//! Moving a task within or between status columns. The rules are checked here. The write, and the
//! key that orders the task in its new place, live in persistence.

use crate::{
    error::DomainError,
    ids::{ProjectId, TaskId},
    status::Status,
};

/// A task placed at `index` in the `to` column of one project. The index counts the column's
/// other tasks, so it's the same index the drop placeholder showed. A move that keeps the status
/// is a reorder within the column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaskMove {
    task: TaskId,
    project: ProjectId,
    from: Status,
    to: Status,
    index: usize,
}

impl TaskMove {
    /// `visible` is the statuses shown on the project. Moving into a hidden column is rejected.
    pub fn new(
        task: TaskId,
        project: ProjectId,
        from: Status,
        to: Status,
        index: usize,
        visible: &[Status],
    ) -> Result<Self, DomainError> {
        if !visible.contains(&to) {
            return Err(DomainError::StatusNotVisible {
                status: to.as_str().to_string(),
            });
        }
        Ok(Self {
            task,
            project,
            from,
            to,
            index,
        })
    }

    pub fn task(&self) -> TaskId {
        self.task
    }

    pub fn project(&self) -> ProjectId {
        self.project
    }

    pub fn from(&self) -> Status {
        self.from
    }

    pub fn to(&self) -> Status {
        self.to
    }

    pub fn index(&self) -> usize {
        self.index
    }
}
