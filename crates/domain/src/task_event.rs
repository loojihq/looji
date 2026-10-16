//! The history of a task as the detail panel shows it: when it was created, what was edited, and
//! where it moved. Each entry is written in the same transaction as the change it describes.

use crate::{task::Task, time::Timestamp};

/// What happened to a task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskEventKind {
    Created,
    /// The named fields changed, in the order the panel and dialog list them.
    Edited {
        fields: Vec<String>,
    },
    StatusChanged {
        from: crate::status::Status,
        to: crate::status::Status,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskEvent {
    pub at: Timestamp,
    pub kind: TaskEventKind,
}

/// The editable fields that differ between two versions of a task, by name. Status isn't listed,
/// because a status change is a move with its own history entry.
pub fn changed_fields(before: &Task, after: &Task) -> Vec<&'static str> {
    let mut changed = Vec::new();
    if before.title() != after.title() {
        changed.push("title");
    }
    if before.description() != after.description() {
        changed.push("description");
    }
    if before.priority() != after.priority() {
        changed.push("priority");
    }
    if before.estimate() != after.estimate() {
        changed.push("estimate");
    }
    if before.start_on() != after.start_on() {
        changed.push("start");
    }
    if before.due_on() != after.due_on() {
        changed.push("due");
    }
    if before.tags() != after.tags() {
        changed.push("tags");
    }
    if before.assignee_id() != after.assignee_id() {
        changed.push("assignee");
    }
    changed
}

#[cfg(test)]
mod tests {
    // Not `use super::*`: gpui's `test` macro would shadow `#[test]`.
    use crate::{
        ids::{ProjectId, TeamMemberId},
        position::Position,
        status::Status,
        task::Estimate,
        task::Task,
        task_event::changed_fields,
    };

    fn task() -> Task {
        Task::new(
            ProjectId::new(),
            "Write copy",
            Status::Todo,
            Position::new("a0").unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn an_unchanged_task_has_no_changed_fields() {
        let before = task();
        let after = before.clone();
        assert!(changed_fields(&before, &after).is_empty());
    }

    #[test]
    fn each_edited_field_is_named_once() {
        let before = task();
        let mut after = before.clone();
        after.rename("Write more copy").unwrap();
        after.set_estimate(Some(Estimate::from_minutes(30).unwrap()));
        after.set_tags(vec!["web".to_string()]);
        assert_eq!(
            changed_fields(&before, &after),
            ["title", "estimate", "tags"]
        );
    }

    #[test]
    fn a_status_change_is_not_a_field_edit() {
        let before = task();
        let mut after = before.clone();
        after.set_status(Status::Done);
        assert!(changed_fields(&before, &after).is_empty());
    }

    #[test]
    fn an_assignee_change_is_named_including_being_cleared() {
        let before = task();
        let mut assigned = before.clone();
        assigned.assign(Some(TeamMemberId::new()));
        assert_eq!(changed_fields(&before, &assigned), ["assignee"]);
        assert_eq!(changed_fields(&assigned, &before), ["assignee"]);
    }
}
