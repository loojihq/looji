//! The board's filter: which tasks a project shows, by status, priority, assignee, and tag. It's plain
//! data with no window or database in it, so the rule is tested on its own.
//!
//! Across facets, a task must pass every facet that has a value selected (AND). Within a facet, a task
//! passes if it matches any of the values selected (OR). Tags are the exception: a task can match
//! any of the selected tags, or all of them, as the filter's `tag_match` says. An empty facet puts no
//! constraint on the task.

use crate::{
    ids::TeamMemberId,
    status::Status,
    task::Task,
    task_fields::{Priority, normalize_tag},
};

/// How selected tags combine: a task needs any one of them, or every one of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TagMatch {
    #[default]
    Any,
    All,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TaskFilter {
    pub statuses: Vec<Status>,
    /// `None` selects tasks with no priority.
    pub priorities: Vec<Option<Priority>>,
    /// `None` selects unassigned tasks.
    pub assignees: Vec<Option<TeamMemberId>>,
    /// Tags in stored form.
    pub tags: Vec<String>,
    pub tag_match: TagMatch,
}

impl TaskFilter {
    /// How many of the four facets have a value selected. Each facet counts once, however many of its
    /// values are chosen.
    pub fn active_facets(&self) -> usize {
        [
            !self.statuses.is_empty(),
            !self.priorities.is_empty(),
            !self.assignees.is_empty(),
            !self.tags.is_empty(),
        ]
        .into_iter()
        .filter(|active| *active)
        .count()
    }

    pub fn is_empty(&self) -> bool {
        self.active_facets() == 0
    }

    /// Whether a task passes every facet that has a value selected.
    pub fn matches(&self, task: &Task) -> bool {
        (self.statuses.is_empty() || self.statuses.contains(&task.status()))
            && (self.priorities.is_empty() || self.priorities.contains(&task.priority()))
            && (self.assignees.is_empty() || self.assignees.contains(&task.assignee_id()))
            && (self.tags.is_empty() || self.matches_tags(task))
    }

    fn matches_tags(&self, task: &Task) -> bool {
        let has = |tag: &String| task.tags().contains(tag);
        match self.tag_match {
            TagMatch::Any => self.tags.iter().any(has),
            TagMatch::All => self.tags.iter().all(has),
        }
    }

    pub fn toggle_status(&mut self, status: Status) {
        toggle(&mut self.statuses, status);
    }

    pub fn toggle_priority(&mut self, priority: Option<Priority>) {
        toggle(&mut self.priorities, priority);
    }

    pub fn toggle_assignee(&mut self, assignee: Option<TeamMemberId>) {
        toggle(&mut self.assignees, assignee);
    }

    /// Adds a tag, normalised the same way tags are stored, or removes it if it's already selected.
    /// A tag that doesn't normalise is ignored.
    pub fn toggle_tag(&mut self, tag: &str) {
        if let Ok(tag) = normalize_tag(tag) {
            toggle(&mut self.tags, tag);
        }
    }

    /// Resets every facet and the tag mode. Clearing is always a full reset.
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// Drops selected values that no longer exist: a team member who was removed, or a tag no task
    /// uses any more. Without this, the bar would show a selection that matches nothing. Returns
    /// whether anything was dropped.
    pub fn prune(&mut self, members: &[TeamMemberId], tags_in_use: &[String]) -> bool {
        let before = (self.assignees.len(), self.tags.len());
        self.assignees
            .retain(|assignee| assignee.is_none_or(|id| members.contains(&id)));
        self.tags.retain(|tag| tags_in_use.contains(tag));
        before != (self.assignees.len(), self.tags.len())
    }
}

/// Adds the value if it's missing, or removes it if it's there.
fn toggle<T: PartialEq>(values: &mut Vec<T>, value: T) {
    match values.iter().position(|existing| *existing == value) {
        Some(index) => {
            values.remove(index);
        }
        None => values.push(value),
    }
}

#[cfg(test)]
mod tests {
    // Not `use super::*`: gpui's `test` macro would shadow `#[test]`.
    use crate::{
        ids::{ProjectId, TeamMemberId},
        position::Position,
        status::Status,
        task::Task,
        task_fields::Priority,
        task_filter::{TagMatch, TaskFilter},
    };

    fn task(status: Status, priority: Option<Priority>, tags: &[&str]) -> Task {
        let mut task = Task::new(
            ProjectId::new(),
            "Task",
            status,
            Position::new("a0").unwrap(),
        )
        .unwrap();
        task.set_priority(priority);
        for tag in tags {
            task.add_tag(tag).unwrap();
        }
        task
    }

    #[test]
    fn an_empty_filter_shows_every_task() {
        let filter = TaskFilter::default();
        assert!(filter.is_empty());
        assert!(filter.matches(&task(Status::Backlog, None, &[])));
        assert!(filter.matches(&task(Status::Done, Some(Priority::High), &["x"])));
    }

    #[test]
    fn one_status_facet_keeps_only_that_status() {
        let mut filter = TaskFilter::default();
        filter.toggle_status(Status::Todo);
        assert!(filter.matches(&task(Status::Todo, None, &[])));
        assert!(!filter.matches(&task(Status::Done, None, &[])));
    }

    #[test]
    fn several_values_in_one_facet_are_or() {
        let mut filter = TaskFilter::default();
        filter.toggle_status(Status::Todo);
        filter.toggle_status(Status::InProgress);
        assert!(filter.matches(&task(Status::Todo, None, &[])));
        assert!(filter.matches(&task(Status::InProgress, None, &[])));
        assert!(!filter.matches(&task(Status::Done, None, &[])));
    }

    #[test]
    fn facets_are_and() {
        let mut filter = TaskFilter::default();
        filter.toggle_priority(Some(Priority::High));
        filter.toggle_tag("bug");
        assert!(filter.matches(&task(Status::Todo, Some(Priority::High), &["bug"])));
        assert!(!filter.matches(&task(Status::Todo, Some(Priority::Low), &["bug"])));
        assert!(!filter.matches(&task(Status::Todo, Some(Priority::High), &["ui"])));
    }

    #[test]
    fn no_priority_is_its_own_choice() {
        let mut filter = TaskFilter::default();
        filter.toggle_priority(None);
        assert!(filter.matches(&task(Status::Todo, None, &[])));
        assert!(!filter.matches(&task(Status::Todo, Some(Priority::Low), &[])));
    }

    #[test]
    fn no_assignee_selects_unassigned_tasks() {
        let member = TeamMemberId::new();
        let mut filter = TaskFilter::default();
        filter.toggle_assignee(None);
        let mut assigned = task(Status::Todo, None, &[]);
        assigned.assign(Some(member));
        assert!(filter.matches(&task(Status::Todo, None, &[])));
        assert!(!filter.matches(&assigned));
    }

    #[test]
    fn a_member_filter_matches_only_that_member() {
        let (first, second) = (TeamMemberId::new(), TeamMemberId::new());
        let mut filter = TaskFilter::default();
        filter.toggle_assignee(Some(first));
        let mut mine = task(Status::Todo, None, &[]);
        mine.assign(Some(first));
        let mut theirs = task(Status::Todo, None, &[]);
        theirs.assign(Some(second));
        assert!(filter.matches(&mine));
        assert!(!filter.matches(&theirs));
    }

    #[test]
    fn any_tag_mode_needs_one_selected_tag() {
        let mut filter = TaskFilter::default();
        filter.toggle_tag("bug");
        filter.toggle_tag("ui");
        assert!(filter.matches(&task(Status::Todo, None, &["ui"])));
        assert!(!filter.matches(&task(Status::Todo, None, &["web"])));
    }

    #[test]
    fn all_tag_mode_needs_every_selected_tag() {
        let mut filter = TaskFilter {
            tag_match: TagMatch::All,
            ..Default::default()
        };
        filter.toggle_tag("bug");
        filter.toggle_tag("ui");
        assert!(filter.matches(&task(Status::Todo, None, &["bug", "ui"])));
        assert!(!filter.matches(&task(Status::Todo, None, &["ui"])));
    }

    #[test]
    fn tag_selections_are_normalised_so_case_does_not_matter() {
        let mut filter = TaskFilter::default();
        filter.toggle_tag("  BUG ");
        assert_eq!(filter.tags, ["bug"]);
        assert!(filter.matches(&task(Status::Todo, None, &["bug"])));
    }

    #[test]
    fn toggling_a_selected_value_removes_it() {
        let mut filter = TaskFilter::default();
        filter.toggle_status(Status::Todo);
        filter.toggle_status(Status::Todo);
        assert!(filter.is_empty());
    }

    #[test]
    fn the_count_is_facets_not_values() {
        let mut filter = TaskFilter::default();
        filter.toggle_status(Status::Todo);
        filter.toggle_status(Status::Done);
        filter.toggle_priority(Some(Priority::High));
        assert_eq!(filter.active_facets(), 2);
    }

    #[test]
    fn clearing_resets_every_facet_and_the_tag_mode() {
        let mut filter = TaskFilter {
            tag_match: TagMatch::All,
            ..Default::default()
        };
        filter.toggle_status(Status::Todo);
        filter.toggle_tag("bug");
        filter.clear();
        assert_eq!(filter, TaskFilter::default());
        assert_eq!(filter.active_facets(), 0);
    }

    #[test]
    fn pruning_drops_removed_members_and_unused_tags() {
        let kept = TeamMemberId::new();
        let gone = TeamMemberId::new();
        let mut filter = TaskFilter::default();
        filter.toggle_assignee(Some(kept));
        filter.toggle_assignee(Some(gone));
        filter.toggle_assignee(None);
        filter.toggle_tag("bug");
        filter.toggle_tag("old");

        let changed = filter.prune(&[kept], &["bug".to_string()]);

        assert!(changed);
        assert_eq!(filter.assignees, [Some(kept), None]);
        assert_eq!(filter.tags, ["bug"]);
    }

    #[test]
    fn pruning_a_current_filter_changes_nothing() {
        let member = TeamMemberId::new();
        let mut filter = TaskFilter::default();
        filter.toggle_assignee(Some(member));
        filter.toggle_tag("bug");
        assert!(!filter.prune(&[member], &["bug".to_string()]));
    }
}
