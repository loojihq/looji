//! Groups a project's tasks into the board's status columns.

use crate::{status::Status, task::Task};

/// One board column: a status and the tasks in it, in the order they were given.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusColumn<'a> {
    pub status: Status,
    pub tasks: Vec<&'a Task>,
}

impl StatusColumn<'_> {
    /// The count shown in the column header. Derived from the tasks, so it can't drift.
    pub fn count(&self) -> usize {
        self.tasks.len()
    }
}

/// Puts each task in the column for its status.
///
/// Always returns one column per status, in `Status::ALL` order, including empty ones. Every
/// task appears in exactly one column.
pub fn status_columns<'a>(tasks: &'a [Task], visible: &[Status]) -> Vec<StatusColumn<'a>> {
    visible
        .iter()
        .copied()
        .map(|status| StatusColumn {
            status,
            tasks: tasks
                .iter()
                .filter(|task| task.status() == status)
                .collect(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ids::ProjectId, position::Position};

    fn task(status: Status, title: &str) -> Task {
        Task::new(
            ProjectId::new(),
            title,
            status,
            Position::new("a0").unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn always_returns_all_four_columns_in_order() {
        let columns = status_columns(&[], &Status::ALL);
        let statuses: Vec<Status> = columns.iter().map(|column| column.status).collect();
        assert_eq!(statuses, Status::ALL.to_vec());
    }

    #[test]
    fn empty_columns_are_included_with_a_zero_count() {
        let tasks = [task(Status::Todo, "Write docs")];
        let columns = status_columns(&tasks, &Status::ALL);

        let done = columns
            .iter()
            .find(|column| column.status == Status::Done)
            .unwrap();
        assert_eq!(done.count(), 0);
        assert!(done.tasks.is_empty());
    }

    #[test]
    fn each_task_lands_in_exactly_one_column_matching_its_status() {
        let tasks = [
            task(Status::Backlog, "a"),
            task(Status::Todo, "b"),
            task(Status::Todo, "c"),
            task(Status::InProgress, "d"),
            task(Status::Done, "e"),
        ];

        let columns = status_columns(&tasks, &Status::ALL);

        let total: usize = columns.iter().map(StatusColumn::count).sum();
        assert_eq!(total, tasks.len());
        for column in &columns {
            assert!(
                column
                    .tasks
                    .iter()
                    .all(|task| task.status() == column.status)
            );
        }
        let todo = columns
            .iter()
            .find(|column| column.status == Status::Todo)
            .unwrap();
        assert_eq!(todo.count(), 2);
    }

    #[test]
    fn tasks_keep_the_order_they_were_given() {
        let tasks = [task(Status::Todo, "first"), task(Status::Todo, "second")];
        let columns = status_columns(&tasks, &Status::ALL);
        let titles: Vec<&str> = columns[1].tasks.iter().map(|task| task.title()).collect();
        assert_eq!(titles, vec!["first", "second"]);
    }
}
