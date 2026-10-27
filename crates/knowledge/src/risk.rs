//! Quiet risk notes for a task, from facts the app already has: how long it has sat, when it is due, and whether any
//! commit is about it. The rules are plain and strict so a note is worth reading. Nothing here is applied.

/// Where a task is, as far as a note needs to know.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// Backlog or to do: not started.
    NotStarted,
    /// In progress or in review.
    Active,
    Done,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaskState {
    pub stage: Stage,
    /// Whole days since the task last changed.
    pub idle_days: i64,
    /// Whole days until it is due. Negative once it is overdue.
    pub due_in_days: Option<i64>,
    /// How many commits are about it.
    pub linked_commits: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Note {
    /// Started, and has not moved for this many days.
    Stale { days: i64 },
    /// Past its due date by this many days.
    Overdue { days: i64 },
    /// Due within a few days, started or not, and no commit is about it.
    DueSoonNoCommits { days: i64 },
}

/// A started task that has not changed in this many days is stale.
const STALE_DAYS: i64 = 7;
/// Due within this many days counts as soon.
const SOON_DAYS: i64 = 3;

impl Note {
    pub fn text(self) -> String {
        match self {
            Note::Stale { days } => format!("Hasn't moved in {days} days"),
            Note::Overdue { days: 1 } => "Overdue by a day".to_string(),
            Note::Overdue { days } => format!("Overdue by {days} days"),
            Note::DueSoonNoCommits { days: 0 } => "Due today, with no commits about it".to_string(),
            Note::DueSoonNoCommits { days: 1 } => {
                "Due tomorrow, with no commits about it".to_string()
            }
            Note::DueSoonNoCommits { days } => {
                format!("Due in {days} days, with no commits about it")
            }
        }
    }
}

/// The notes a task's state supports, most urgent first. A finished task has none.
pub fn notes(state: &TaskState) -> Vec<Note> {
    if state.stage == Stage::Done {
        return Vec::new();
    }
    let mut found = Vec::new();
    match state.due_in_days {
        Some(days) if days < 0 => found.push(Note::Overdue { days: -days }),
        Some(days) if days <= SOON_DAYS && state.linked_commits == 0 => {
            found.push(Note::DueSoonNoCommits { days });
        }
        _ => {}
    }
    if state.stage == Stage::Active && state.idle_days >= STALE_DAYS {
        found.push(Note::Stale {
            days: state.idle_days,
        });
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(stage: Stage) -> TaskState {
        TaskState {
            stage,
            idle_days: 0,
            due_in_days: None,
            linked_commits: 0,
        }
    }

    #[test]
    fn a_finished_task_has_no_notes() {
        let done = TaskState {
            idle_days: 30,
            due_in_days: Some(-5),
            ..state(Stage::Done)
        };
        assert!(notes(&done).is_empty());
    }

    #[test]
    fn started_work_that_sat_a_week_is_stale() {
        let stale = TaskState {
            idle_days: 9,
            ..state(Stage::Active)
        };
        assert_eq!(notes(&stale), vec![Note::Stale { days: 9 }]);
        let fresh = TaskState {
            idle_days: 6,
            ..state(Stage::Active)
        };
        assert!(notes(&fresh).is_empty());
    }

    #[test]
    fn work_not_started_is_never_stale() {
        let waiting = TaskState {
            idle_days: 60,
            ..state(Stage::NotStarted)
        };
        assert!(notes(&waiting).is_empty());
    }

    #[test]
    fn an_overdue_task_says_how_late() {
        let late = TaskState {
            due_in_days: Some(-2),
            linked_commits: 4,
            ..state(Stage::Active)
        };
        assert_eq!(notes(&late), vec![Note::Overdue { days: 2 }]);
        assert_eq!(Note::Overdue { days: 1 }.text(), "Overdue by a day");
    }

    #[test]
    fn due_soon_is_only_a_risk_without_commits() {
        let bare = TaskState {
            due_in_days: Some(2),
            ..state(Stage::NotStarted)
        };
        assert_eq!(notes(&bare), vec![Note::DueSoonNoCommits { days: 2 }]);
        let moving = TaskState {
            linked_commits: 1,
            ..bare
        };
        assert!(notes(&moving).is_empty());
        let far = TaskState {
            due_in_days: Some(10),
            ..bare
        };
        assert!(notes(&far).is_empty());
    }

    #[test]
    fn the_most_urgent_note_comes_first() {
        let both = TaskState {
            idle_days: 8,
            due_in_days: Some(-1),
            ..state(Stage::Active)
        };
        assert_eq!(
            notes(&both),
            vec![Note::Overdue { days: 1 }, Note::Stale { days: 8 }]
        );
    }
}
