//! Moving a task to another status column. The rules are checked here. The write, and the history
//! entry that goes with it, live in persistence.

use crate::{
    error::DomainError,
    ids::{ProjectId, TaskId},
    status::Status,
};

/// A task moving from one status to another within one project.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaskMove {
    task: TaskId,
    project: ProjectId,
    from: Status,
    to: Status,
}

impl TaskMove {
    /// `visible` is the statuses shown on the project. Moving into a hidden column is rejected,
    /// and so is a move to the column the task is already in.
    pub fn new(
        task: TaskId,
        project: ProjectId,
        from: Status,
        to: Status,
        visible: &[Status],
    ) -> Result<Self, DomainError> {
        if from == to {
            return Err(DomainError::SameStatus);
        }
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
}
