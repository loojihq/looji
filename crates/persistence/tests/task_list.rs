use std::collections::HashSet;

use looji_domain::{
    PAGE_SIZE, PageCursor, Position, Priority, Project, ProjectId, ProjectRepository, Status,
    TagMatch, Task, TaskFilter, TaskId, TaskQuery, TaskRepository, TeamMember,
    TeamMemberRepository, Workspace, WorkspaceRepository,
};
use looji_persistence::SqliteStore;
use tempfile::TempDir;

const ALL_STATUSES: [Status; 5] = [
    Status::Backlog,
    Status::Todo,
    Status::InProgress,
    Status::InReview,
    Status::Done,
];

fn store_at(dir: &TempDir) -> SqliteStore {
    SqliteStore::open(&dir.path().join("looji.db")).unwrap()
}

fn project(store: &SqliteStore) -> Project {
    let workspace = Workspace::new("Work").unwrap();
    WorkspaceRepository::save(store, &workspace).unwrap();
    let project = Project::new(workspace.id(), "looji").unwrap();
    ProjectRepository::save(store, &project).unwrap();
    project
}

/// Seeds `count` tasks with varied fields. Positions are unique and sort in creation order.
fn seed(store: &SqliteStore, project: &Project, count: usize) -> Vec<Task> {
    let member = TeamMember::new(
        project.id(),
        "Dana",
        looji_domain::MemberColour::for_new_member(0),
    )
    .unwrap();
    TeamMemberRepository::save(store, &member).unwrap();
    (0..count)
        .map(|index| {
            let mut task = Task::new(
                project.id(),
                &format!("Task {index} copy"),
                ALL_STATUSES[index % ALL_STATUSES.len()],
                Position::new(format!("a{index:06}")).unwrap(),
            )
            .unwrap();
            task.set_priority(match index % 4 {
                0 => None,
                1 => Some(Priority::Low),
                2 => Some(Priority::Medium),
                _ => Some(Priority::High),
            });
            if index % 3 == 0 {
                task.add_tag("bug").unwrap();
            }
            if index % 5 == 0 {
                task.add_tag("ui").unwrap();
            }
            if index % 7 == 0 {
                task.add_tag("École").unwrap();
            }
            if index % 6 == 0 {
                task.assign(Some(member.id()));
            }
            if index % 11 == 0 {
                task.set_description("the Ünïcode launch notes");
            }
            TaskRepository::save(store, &task).unwrap();
            task
        })
        .collect()
}

fn all_statuses_query(filter: TaskFilter, search: &str) -> TaskQuery {
    TaskQuery::new(ALL_STATUSES.to_vec(), filter, search)
}

/// Walks every page of a query from the start, returning each page's task IDs.
fn walk(store: &SqliteStore, project: ProjectId, query: &TaskQuery) -> Vec<Vec<TaskId>> {
    let mut pages = Vec::new();
    let mut cursor: Option<PageCursor> = None;
    loop {
        let page = store
            .list_page(project, query, cursor.as_ref(), PAGE_SIZE)
            .unwrap();
        pages.push(page.tasks.iter().map(Task::id).collect());
        match page.tasks.last() {
            Some(last) if page.has_next => cursor = Some(PageCursor::after(last)),
            _ => break,
        }
    }
    pages
}

#[test]
fn page_sizes_at_the_boundaries_have_no_phantom_rows() {
    let dir = TempDir::new().unwrap();
    let store = store_at(&dir);
    let project = project(&store);
    let query = all_statuses_query(TaskFilter::default(), "");
    let mut seeded = 0;

    for count in [0, 1, 49, 50, 51, 99, 100, 137] {
        seed_up_to(&store, &project, &mut seeded, count);
        let pages = walk(&store, project.id(), &query);
        let expected_pages = count.div_ceil(PAGE_SIZE).max(1);
        if count == 0 {
            assert_eq!(pages.len(), 1, "an empty project is one empty page");
            assert!(pages[0].is_empty());
            continue;
        }
        assert_eq!(pages.len(), expected_pages, "page count for {count} tasks");
        for (index, page) in pages.iter().enumerate() {
            let expected_rows = if index + 1 == expected_pages {
                count - index * PAGE_SIZE
            } else {
                PAGE_SIZE
            };
            assert_eq!(page.len(), expected_rows, "rows on page {index} of {count}");
        }
        let total = store
            .list_page(project.id(), &query, None, PAGE_SIZE)
            .unwrap()
            .total;
        assert_eq!(total, count as u64);
    }
}

/// Adds tasks until the project has `target`. Keeps `seeded` as the count so far.
fn seed_up_to(store: &SqliteStore, project: &Project, seeded: &mut usize, target: usize) {
    if target > *seeded {
        let extra: Vec<Task> = (*seeded..target)
            .map(|index| {
                let task = Task::new(
                    project.id(),
                    &format!("Task {index} copy"),
                    ALL_STATUSES[index % ALL_STATUSES.len()],
                    Position::new(format!("a{index:06}")).unwrap(),
                )
                .unwrap();
                TaskRepository::save(store, &task).unwrap();
                task
            })
            .collect();
        assert_eq!(extra.len(), target - *seeded);
        *seeded = target;
    }
}

#[test]
fn a_full_walk_returns_each_board_task_once() {
    let dir = TempDir::new().unwrap();
    let store = store_at(&dir);
    let project = project(&store);
    let tasks = seed(&store, &project, 137);
    let query = all_statuses_query(TaskFilter::default(), "");

    let walked: Vec<TaskId> = walk(&store, project.id(), &query)
        .into_iter()
        .flatten()
        .collect();

    let board: HashSet<TaskId> = tasks.iter().map(Task::id).collect();
    let unique: HashSet<TaskId> = walked.iter().copied().collect();
    assert_eq!(walked.len(), unique.len(), "no task shows twice");
    assert_eq!(unique, board, "the pages hold exactly the board's tasks");
}

#[test]
fn editing_rows_during_a_walk_skips_and_repeats_nothing() {
    let dir = TempDir::new().unwrap();
    let store = store_at(&dir);
    let project = project(&store);
    let mut tasks = seed(&store, &project, 137);
    let query = all_statuses_query(TaskFilter::default(), "");

    let first = store
        .list_page(project.id(), &query, None, PAGE_SIZE)
        .unwrap();
    let before_walk: Vec<TaskId> = walk(&store, project.id(), &query)
        .into_iter()
        .flatten()
        .collect();
    let cursor = PageCursor::after(first.tasks.last().unwrap());

    // Edit a row already seen, and one further on that hasn't been reached. Neither moves in the order.
    let seen = tasks[0].id();
    let unseen = tasks[120].id();
    for id in [seen, unseen] {
        let task = tasks.iter_mut().find(|task| task.id() == id).unwrap();
        task.rename("Renamed while paging").unwrap();
        TaskRepository::save(&store, task).unwrap();
    }

    let mut walked: Vec<TaskId> = first.tasks.iter().map(Task::id).collect();
    let mut next = Some(cursor);
    while let Some(after) = next {
        let page = store
            .list_page(project.id(), &query, Some(&after), PAGE_SIZE)
            .unwrap();
        walked.extend(page.tasks.iter().map(Task::id));
        next = page
            .tasks
            .last()
            .filter(|_| page.has_next)
            .map(PageCursor::after);
    }

    assert_eq!(
        walked, before_walk,
        "the same rows, in the same order, with no skips or repeats"
    );
}

#[test]
fn moving_a_row_changes_only_where_it_appears_and_never_repeats_it() {
    // Documented expectation: a task's sort key is part of the order, so moving it moves it in the list.
    // A task moved past the cursor is seen later; a task moved behind it is not seen again.
    let dir = TempDir::new().unwrap();
    let store = store_at(&dir);
    let project = project(&store);
    let mut tasks = seed(&store, &project, 60);
    let query = all_statuses_query(TaskFilter::default(), "");
    let first = store
        .list_page(project.id(), &query, None, PAGE_SIZE)
        .unwrap();
    let cursor = PageCursor::after(first.tasks.last().unwrap());

    let moved = tasks[0].id();
    let task = tasks.iter_mut().find(|task| task.id() == moved).unwrap();
    task.set_position(Position::new("zz999999").unwrap());
    TaskRepository::save(&store, task).unwrap();

    let rest = store
        .list_page(project.id(), &query, Some(&cursor), PAGE_SIZE)
        .unwrap();
    // The one repeat the order allows is the moved task itself, which was shown before and now sorts past
    // the cursor. No other row already shown comes back.
    let seen: HashSet<TaskId> = first
        .tasks
        .iter()
        .map(Task::id)
        .filter(|id| *id != moved)
        .collect();
    assert!(
        rest.tasks.iter().all(|task| !seen.contains(&task.id())),
        "only the moved task may come back"
    );
    assert_eq!(
        rest.tasks.last().unwrap().id(),
        moved,
        "the moved task now sorts last"
    );
}

#[test]
fn tasks_that_share_a_sort_key_are_each_shown_once() {
    let dir = TempDir::new().unwrap();
    let store = store_at(&dir);
    let project = project(&store);
    // Two tasks in different columns can share a position key. The id breaks the tie.
    for status in [Status::InReview, Status::Done] {
        let task = Task::new(
            project.id(),
            "Same key",
            status,
            Position::new("a000001").unwrap(),
        )
        .unwrap();
        TaskRepository::save(&store, &task).unwrap();
    }
    seed(&store, &project, 3);
    let query = all_statuses_query(TaskFilter::default(), "");

    let walked: Vec<TaskId> = walk(&store, project.id(), &query)
        .into_iter()
        .flatten()
        .collect();

    let unique: HashSet<TaskId> = walked.iter().copied().collect();
    assert_eq!(walked.len(), 5);
    assert_eq!(unique.len(), 5);
}

#[test]
fn a_cursor_survives_deletion_of_the_row_it_was_taken_from() {
    let dir = TempDir::new().unwrap();
    let store = store_at(&dir);
    let project = project(&store);
    let tasks = seed(&store, &project, 60);
    let query = all_statuses_query(TaskFilter::default(), "");
    let first = store
        .list_page(project.id(), &query, None, PAGE_SIZE)
        .unwrap();
    let cursor = PageCursor::after(first.tasks.last().unwrap());

    TaskRepository::delete(&store, first.tasks.last().unwrap().id()).unwrap();

    let rest = store
        .list_page(project.id(), &query, Some(&cursor), PAGE_SIZE)
        .unwrap();
    let expected: Vec<TaskId> = tasks[50..]
        .iter()
        .map(Task::id)
        .filter(|id| *id != first.tasks.last().unwrap().id())
        .collect();
    let shown: Vec<TaskId> = rest.tasks.iter().map(Task::id).collect();
    assert_eq!(shown, expected);
}

/// The filters and searches the count is checked under, alone and combined.
fn cases() -> Vec<(&'static str, TaskQuery)> {
    let mut by_status = TaskFilter::default();
    by_status.toggle_status(Status::Todo);
    by_status.toggle_status(Status::InProgress);
    let mut no_priority = TaskFilter::default();
    no_priority.toggle_priority(None);
    let mut high_or_low = TaskFilter::default();
    high_or_low.toggle_priority(Some(Priority::High));
    high_or_low.toggle_priority(Some(Priority::Low));
    let mut unassigned = TaskFilter::default();
    unassigned.toggle_assignee(None);
    let mut any_tag = TaskFilter::default();
    any_tag.toggle_tag("bug");
    any_tag.toggle_tag("ui");
    let mut all_tags = TaskFilter {
        tag_match: TagMatch::All,
        ..Default::default()
    };
    all_tags.toggle_tag("bug");
    all_tags.toggle_tag("ui");
    let mut combined = TaskFilter::default();
    combined.toggle_priority(Some(Priority::High));
    combined.toggle_tag("bug");

    vec![
        ("nothing", all_statuses_query(TaskFilter::default(), "")),
        ("status", all_statuses_query(by_status, "")),
        ("no priority", all_statuses_query(no_priority, "")),
        ("high or low", all_statuses_query(high_or_low, "")),
        ("unassigned", all_statuses_query(unassigned, "")),
        ("any tag", all_statuses_query(any_tag, "")),
        ("all tags", all_statuses_query(all_tags, "")),
        (
            "search",
            all_statuses_query(TaskFilter::default(), "copy 7"),
        ),
        (
            "search title case",
            all_statuses_query(TaskFilter::default(), "ÉCOLE"),
        ),
        (
            "search description",
            all_statuses_query(TaskFilter::default(), "unicode"),
        ),
        (
            "special characters",
            all_statuses_query(TaskFilter::default(), "%"),
        ),
        (
            "regex-looking",
            all_statuses_query(TaskFilter::default(), ".* ("),
        ),
        ("combined", all_statuses_query(combined, "task")),
    ]
}

#[test]
fn the_list_total_matches_the_board_for_every_filter_and_search() {
    let dir = TempDir::new().unwrap();
    let store = store_at(&dir);
    let project = project(&store);
    seed(&store, &project, 137);
    let board = TaskRepository::list_for_project(&store, project.id()).unwrap();

    for (name, query) in cases() {
        let on_board = board.iter().filter(|task| query.matches(task)).count() as u64;
        let listed = store
            .list_page(project.id(), &query, None, PAGE_SIZE)
            .unwrap()
            .total;
        assert_eq!(listed, on_board, "total for {name}");
    }
}

#[test]
fn a_walk_under_a_filter_shows_exactly_the_matching_board_tasks() {
    let dir = TempDir::new().unwrap();
    let store = store_at(&dir);
    let project = project(&store);
    seed(&store, &project, 137);
    let board = TaskRepository::list_for_project(&store, project.id()).unwrap();

    for (name, query) in cases() {
        let expected: HashSet<TaskId> = board
            .iter()
            .filter(|task| query.matches(task))
            .map(Task::id)
            .collect();
        let walked: Vec<TaskId> = walk(&store, project.id(), &query)
            .into_iter()
            .flatten()
            .collect();
        assert_eq!(walked.len(), expected.len(), "rows for {name}");
        assert_eq!(
            walked.into_iter().collect::<HashSet<_>>(),
            expected,
            "tasks for {name}"
        );
    }
}

#[test]
fn the_page_query_walks_the_position_index_without_sorting() {
    let dir = TempDir::new().unwrap();
    let store = store_at(&dir);
    let project = project(&store);
    seed(&store, &project, 60);
    let query = all_statuses_query(TaskFilter::default(), "");
    let first = store
        .list_page(project.id(), &query, None, PAGE_SIZE)
        .unwrap();
    let cursor = PageCursor::after(first.tasks.last().unwrap());

    for after in [None, Some(&cursor)] {
        let plan = store
            .list_page_query_plan(project.id(), &query, after)
            .unwrap();
        let text = plan.join("\n");
        assert!(
            text.contains("USING INDEX tasks_by_project_position"),
            "the page should use the position index:\n{text}"
        );
        assert!(
            !text.contains("TEMP B-TREE"),
            "the page should not sort:\n{text}"
        );
    }
}

#[test]
fn special_characters_in_a_search_are_text_and_never_an_error() {
    let dir = TempDir::new().unwrap();
    let store = store_at(&dir);
    let project = project(&store);
    seed(&store, &project, 40);
    let board = TaskRepository::list_for_project(&store, project.id()).unwrap();

    for query_text in [
        "%", "_", "\"", "'", "\\", "*", "(", "[", "^", ".*", "-", ":", "🚀", "a'b\"c",
    ] {
        let query = all_statuses_query(TaskFilter::default(), query_text);
        let listed = store.list_page(project.id(), &query, None, PAGE_SIZE);
        assert!(listed.is_ok(), "query {query_text:?} must not fail");
        let on_board = board.iter().filter(|task| query.matches(task)).count() as u64;
        assert_eq!(listed.unwrap().total, on_board, "total for {query_text:?}");
    }
}

#[test]
fn a_project_with_no_visible_statuses_shows_nothing() {
    let dir = TempDir::new().unwrap();
    let store = store_at(&dir);
    let project = project(&store);
    seed(&store, &project, 10);
    let query = TaskQuery::new(Vec::new(), TaskFilter::default(), "");

    let page = store
        .list_page(project.id(), &query, None, PAGE_SIZE)
        .unwrap();

    assert!(page.tasks.is_empty());
    assert_eq!(page.total, 0);
}

#[test]
fn list_pages_carry_their_tags_and_members_through_the_database() {
    let dir = TempDir::new().unwrap();
    let store = store_at(&dir);
    let project = project(&store);
    let tasks = seed(&store, &project, 7);
    let query = all_statuses_query(TaskFilter::default(), "");
    let page = store
        .list_page(project.id(), &query, None, PAGE_SIZE)
        .unwrap();

    for listed in &page.tasks {
        let original = tasks.iter().find(|task| task.id() == listed.id()).unwrap();
        assert_eq!(listed.tags(), original.tags());
        assert_eq!(
            listed.assignee_id().is_some(),
            original.assignee_id().is_some()
        );
    }
}

/// A timing check for the scale target. Ignored by default because it seeds 10,000 tasks. Run with
/// `cargo test --test task_list -- --ignored --nocapture`.
#[test]
#[ignore]
fn a_page_costs_about_the_same_deep_in_a_large_list() {
    let dir = TempDir::new().unwrap();
    let store = store_at(&dir);
    let project = project(&store);
    seed(&store, &project, 10_000);
    let query = all_statuses_query(TaskFilter::default(), "");

    let timed = |after: Option<&PageCursor>| {
        let start = std::time::Instant::now();
        let page = store
            .list_page(project.id(), &query, after, PAGE_SIZE)
            .unwrap();
        (start.elapsed(), page)
    };
    let (first_time, first) = timed(None);
    let mut cursor = None;
    let mut deep = first;
    for _ in 0..149 {
        let after = PageCursor::after(deep.tasks.last().unwrap());
        cursor = Some(after.clone());
        deep = store
            .list_page(project.id(), &query, Some(&after), PAGE_SIZE)
            .unwrap();
    }
    let (deep_time, _) = timed(cursor.as_ref());
    println!("page 1: {first_time:?}, page 150: {deep_time:?}");
}
