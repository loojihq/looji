//! The content of a project report: tasks grouped by status in board order, with a count for each group. Pure. The
//! layout is in the report crate.

use crate::status::Status;

/// One task as it appears in a report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportItem {
    pub title: String,
    pub description: String,
    pub status: Status,
    pub priority: String,
    pub assignee: Option<String>,
    pub due: Option<String>,
}

/// One status group. Every status appears, even with no tasks, so a reader can see that nothing is in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusSection {
    pub status: Status,
    pub label: &'static str,
    pub items: Vec<ReportItem>,
}

impl StatusSection {
    pub fn count(&self) -> usize {
        self.items.len()
    }
}

/// The board's name for a status, as shown in its column header.
pub fn status_label(status: Status) -> &'static str {
    match status {
        Status::Backlog => "Backlog",
        Status::Todo => "To do",
        Status::InProgress => "In progress",
        Status::InReview => "In review",
        Status::Done => "Done",
    }
}

/// Groups the tasks by status, in board order. Each task appears in exactly one section, in the order it was given
/// within its section.
pub fn group_by_status(items: &[ReportItem]) -> Vec<StatusSection> {
    Status::ALL
        .iter()
        .map(|&status| StatusSection {
            status,
            label: status_label(status),
            items: items
                .iter()
                .filter(|item| item.status == status)
                .cloned()
                .collect(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    // Not `use super::*`: gpui's `test` macro would shadow `#[test]`.
    use crate::report::{ReportItem, group_by_status};
    use crate::status::Status;

    fn item(title: &str, status: Status) -> ReportItem {
        ReportItem {
            title: title.to_string(),
            description: String::new(),
            status,
            priority: "medium".to_string(),
            assignee: None,
            due: None,
        }
    }

    #[test]
    fn every_task_appears_exactly_once_in_board_order() {
        let items = vec![
            item("Ship", Status::Done),
            item("Draft", Status::Backlog),
            item("Review", Status::InReview),
            item("Write", Status::Backlog),
        ];

        let sections = group_by_status(&items);

        let order: Vec<Status> = sections.iter().map(|section| section.status).collect();
        assert_eq!(order, Status::ALL.to_vec());
        let titles: Vec<&str> = sections
            .iter()
            .flat_map(|section| section.items.iter().map(|item| item.title.as_str()))
            .collect();
        assert_eq!(titles.len(), items.len());
        for item in &items {
            assert_eq!(
                titles.iter().filter(|title| **title == item.title).count(),
                1
            );
        }
        assert_eq!(sections[0].count(), 2, "backlog holds both backlog tasks");
    }

    #[test]
    fn an_empty_status_still_gets_a_section_with_zero_items() {
        let sections = group_by_status(&[item("Only", Status::Todo)]);

        let in_progress = sections
            .iter()
            .find(|section| section.status == Status::InProgress)
            .expect("in-progress is always present");
        assert_eq!(in_progress.count(), 0);
        assert_eq!(in_progress.label, "In progress");
    }
}
