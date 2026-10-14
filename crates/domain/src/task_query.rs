//! The one definition of which tasks a view shows: the project's visible statuses, the board filter, and
//! the search terms. The board checks cards with [`TaskQuery::matches`], and the list's database query
//! is built from the same values, so the two agree on what matches.
//!
//! Text is compared lowercased one character at a time, with the same rule everywhere. That keeps the
//! database's search function and the in-memory check in step, including for the capital dotted I.

use crate::{ids::TaskId, position::Position, status::Status, task::Task, task_filter::TaskFilter};

/// How many tasks a page of the list holds.
pub const PAGE_SIZE: usize = 50;

/// Lowercases text one character at a time, the rule used for every search comparison.
pub fn lowercase(text: &str) -> String {
    text.chars().flat_map(char::to_lowercase).collect()
}

/// The search terms in a query: split on whitespace, each lowercased. Nothing else is parsed, so
/// punctuation such as `*` or `\` is just text.
pub fn search_terms(query: &str) -> Vec<String> {
    query
        .split_whitespace()
        .map(lowercase)
        .filter(|term| !term.is_empty())
        .collect()
}

/// Whether `text` contains `term`, ignoring case. `term` must already be lowercase, as
/// [`search_terms`] makes it.
pub fn contains_ignoring_case(text: &str, term: &str) -> bool {
    lowercase(text).contains(term)
}

/// Whether one search term appears in a task's title, description, or one of its tags.
pub fn term_matches(task: &Task, term: &str) -> bool {
    contains_ignoring_case(task.title(), term)
        || contains_ignoring_case(task.description(), term)
        || task
            .tags()
            .iter()
            .any(|tag| contains_ignoring_case(tag, term))
}

/// Everything that decides which tasks a view shows.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TaskQuery {
    /// The statuses the project shows. Tasks in any other status are hidden everywhere.
    pub statuses: Vec<Status>,
    pub filter: TaskFilter,
    /// Lowercase search terms. Each must match somewhere on a task (AND).
    pub terms: Vec<String>,
}

impl TaskQuery {
    pub fn new(statuses: Vec<Status>, filter: TaskFilter, search: &str) -> Self {
        Self {
            statuses,
            filter,
            terms: search_terms(search),
        }
    }

    /// Whether a task is shown: its status is visible, it passes the filter, and every search term
    /// matches it.
    pub fn matches(&self, task: &Task) -> bool {
        self.statuses.contains(&task.status())
            && self.filter.matches(task)
            && self.terms.iter().all(|term| term_matches(task, term))
    }
}

/// Where a page ends: the sort key of its last task, with the task id as the tiebreaker. The next page
/// starts after this value, not after a row, so it isn't moved by edits to rows already seen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageCursor {
    pub position: Position,
    pub id: TaskId,
}

impl PageCursor {
    pub fn after(task: &Task) -> Self {
        Self {
            position: task.position().clone(),
            id: task.id(),
        }
    }
}

/// One page of matching tasks, in order, with what the list needs to page on.
#[derive(Debug, Clone, PartialEq)]
pub struct TaskPage {
    pub tasks: Vec<Task>,
    /// Whether more matching tasks follow this page.
    pub has_next: bool,
    /// How many tasks match the query in all, counted in the same read as the page.
    pub total: u64,
}

#[cfg(test)]
mod tests {
    // Not `use super::*`: gpui's `test` macro would shadow `#[test]`.
    use crate::{
        ids::ProjectId,
        position::Position,
        status::Status,
        task::Task,
        task_filter::TaskFilter,
        task_query::{TaskQuery, contains_ignoring_case, search_terms},
    };

    fn task(title: &str, status: Status) -> Task {
        Task::new(
            ProjectId::new(),
            title,
            status,
            Position::new("a0").unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn terms_are_split_and_lowercased_character_by_character() {
        assert_eq!(search_terms("  Foo   BAR "), ["foo", "bar"]);
        assert!(search_terms("   \t ").is_empty());
        assert_eq!(search_terms("İ"), ["i\u{307}"]);
    }

    #[test]
    fn special_characters_are_plain_text() {
        for special in ["*", "(", "\\", "%", "_", "[", ".*", "🚀", "\""] {
            let terms = search_terms(special);
            assert_eq!(terms, [special.to_string()]);
            assert_eq!(
                contains_ignoring_case("nothing here", &terms[0]),
                "nothing here".contains(special),
                "{special:?} is compared as literal text"
            );
        }
        assert!(contains_ignoring_case("a .* b", ".*"));
        assert!(!contains_ignoring_case("anything", ".*"));
    }

    #[test]
    fn a_hidden_status_is_never_matched() {
        let query = TaskQuery::new(vec![Status::Todo], TaskFilter::default(), "");
        assert!(query.matches(&task("x", Status::Todo)));
        assert!(!query.matches(&task("x", Status::Done)));
    }

    #[test]
    fn search_terms_must_all_match() {
        let query = TaskQuery::new(vec![Status::Todo], TaskFilter::default(), "copy launch");
        assert!(query.matches(&task("Write copy for launch", Status::Todo)));
        assert!(!query.matches(&task("Write copy", Status::Todo)));
    }

    #[test]
    fn the_filter_and_the_search_are_both_required() {
        let mut filter = TaskFilter::default();
        filter.toggle_status(Status::Done);
        let query = TaskQuery::new(vec![Status::Todo, Status::Done], filter, "copy");
        assert!(query.matches(&task("copy", Status::Done)));
        assert!(
            !query.matches(&task("copy", Status::Todo)),
            "filter says Done only"
        );
        assert!(
            !query.matches(&task("other", Status::Done)),
            "search says copy"
        );
    }
}
