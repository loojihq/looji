//! The activity history: a record of each change to a task, written in the same transaction as
//! the change itself.

use crate::{ids::ActivityId, ids::TaskId, status::Status, time::Timestamp};

/// One change to a task, as recorded in its history.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaskActivity {
    pub id: ActivityId,
    pub task: TaskId,
    pub kind: ActivityKind,
    pub at: Timestamp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivityKind {
    StatusChanged { from: Status, to: Status },
}
