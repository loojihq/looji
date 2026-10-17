use std::collections::HashMap;

use looji_domain::{
    DueDate, ListColumn, ListSort, ListView, ListViewRepository, PAGE_SIZE, PageCursor, Position,
    Priority, Project, ProjectFilterRepository, ProjectId, ProjectRepository, SortColumn,
    SortDirection, SortValue, Status, TagMatch, Task, TaskFilter, TaskId, TaskQuery,
    TaskRepository, TeamMember, TeamMemberId, TeamMemberRepository, Workspace, WorkspaceRepository,
};
use looji_persistence::{SqliteStore, open_database};
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

/// Seeds tasks with a mix of empty and set values for every sortable field. Tasks 0 to 9 share a due date,
/// so ties are common, and some due dates are empty.
fn seed(store: &SqliteStore, project: &Project, count: usize) -> HashMap<TeamMemberId, String> {
    let dana = TeamMember::new(
        project.id(),
        "Dana",
        looji_domain::MemberColour::for_new_member(0),
    )
    .unwrap();
    let ana = TeamMember::new(
        project.id(),
        "Ana",
        looji_domain::MemberColour::for_new_member(1),
    )
    .unwrap();
    TeamMemberRepository::save(store, &dana).unwrap();
    TeamMemberRepository::save(store, &ana).unwrap();
    for index in 0..count {
        let mut task = Task::new(
            project.id(),
            &format!("Title {}", (index * 7) % count),
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
        if index % 3 != 0 {
            let day = if index < 10 {
                5
            } else {
                (index % 28) as u32 + 1
            };
            task.set_due_on(Some(DueDate::new(2026, 10, day).unwrap()));
        }
        if index % 4 != 0 {
            task.set_estimate_minutes(Some((index as i64 * 13) % 90 + 5))
                .unwrap();
        }
        if index % 5 == 0 {
            task.add_tag("web").unwrap();
        }
        if index % 7 == 0 {
            task.add_tag("bug").unwrap();
            task.add_tag("alpha").unwrap();
        }
        match index % 3 {
            0 => task.assign(Some(dana.id())),
            1 => task.assign(Some(ana.id())),
            _ => {}
        }
        TaskRepository::save(store, &task).unwrap();
    }
    HashMap::from([
        (dana.id(), "Dana".to_string()),
        (ana.id(), "Ana".to_string()),
    ])
}

/// A sort key as the database compares it. Empty values have no key.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Key {
    Text(String),
    Number(i64),
}

fn key_of(task: &Task, column: SortColumn, names: &HashMap<TeamMemberId, String>) -> Option<Key> {
    match column {
        SortColumn::Manual => Some(Key::Text(task.position().as_str().to_string())),
        SortColumn::Title => Some(Key::Text(task.title().to_string())),
        SortColumn::Status => Some(Key::Number(match task.status() {
            Status::Backlog => 0,
            Status::Todo => 1,
            Status::InProgress => 2,
            Status::InReview => 3,
            Status::Done => 4,
        })),
        SortColumn::Priority => task.priority().map(|priority| {
            Key::Number(match priority {
                Priority::Low => 1,
                Priority::Medium => 2,
                Priority::High => 3,
            })
        }),
        SortColumn::Assignee => task
            .assignee_id()
            .and_then(|id| names.get(&id))
            .map(|name| Key::Text(name.clone())),
        SortColumn::Tags => task.tags().first().map(|tag| Key::Text(tag.clone())),
        SortColumn::DueDate => task.due_on().map(|due| Key::Text(due.to_string())),
        SortColumn::Estimate => task
            .estimate()
            .map(|estimate| Key::Number(i64::from(estimate.minutes()))),
    }
}

/// The order the database should produce: values in the direction given, ties by id, and empty values
/// last, ordered by id.
fn expected_order(
    tasks: &[Task],
    sort: ListSort,
    names: &HashMap<TeamMemberId, String>,
) -> Vec<TaskId> {
    let mut valued: Vec<(Key, String, TaskId)> = Vec::new();
    let mut empty: Vec<(String, TaskId)> = Vec::new();
    for task in tasks {
        let id_text = task.id().as_uuid().to_string();
        match key_of(task, sort.column, names) {
            Some(key) => valued.push((key, id_text, task.id())),
            None => empty.push((id_text, task.id())),
        }
    }
    valued.sort_by(|a, b| {
        let by_value = match sort.direction {
            SortDirection::Ascending => a.0.cmp(&b.0),
            SortDirection::Descending => b.0.cmp(&a.0),
        };
        by_value.then(a.1.cmp(&b.1))
    });
    empty.sort_by(|a, b| a.0.cmp(&b.0));
    valued
        .into_iter()
        .map(|(_, _, id)| id)
        .chain(empty.into_iter().map(|(_, id)| id))
        .collect()
}

/// Walks every page of a sorted query, following the cursor each page returns.
fn walk(store: &SqliteStore, project: ProjectId, query: &TaskQuery) -> Vec<TaskId> {
    let mut walked = Vec::new();
    let mut after: Option<PageCursor> = None;
    loop {
        let page = store
            .list_page(project, query, after.as_ref(), PAGE_SIZE)
            .unwrap();
        walked.extend(page.tasks.iter().map(Task::id));
        assert_eq!(
            page.has_next,
            page.next.is_some(),
            "a cursor exists exactly when more follows"
        );
        match page.next {
            Some(next) if page.has_next => after = Some(next),
            _ => break,
        }
    }
    walked
}

fn sorted_query(sort_column: SortColumn, direction: SortDirection) -> TaskQuery {
    TaskQuery::new(ALL_STATUSES.to_vec(), TaskFilter::default(), "").with_sort(ListSort {
        column: sort_column,
        direction,
    })
}

const SORTABLE: [SortColumn; 7] = [
    SortColumn::Title,
    SortColumn::Status,
    SortColumn::Priority,
    SortColumn::Assignee,
    SortColumn::Tags,
    SortColumn::DueDate,
    SortColumn::Estimate,
];

#[test]
fn every_column_sorts_in_both_directions_across_pages_with_empty_values_last() {
    let dir = TempDir::new().unwrap();
    let store = store_at(&dir);
    let project = project(&store);
    let names = seed(&store, &project, 137);
    let tasks = TaskRepository::list_for_project(&store, project.id()).unwrap();

    for column in SORTABLE {
        for direction in [SortDirection::Ascending, SortDirection::Descending] {
            let sort = ListSort { column, direction };
            let walked = walk(&store, project.id(), &sorted_query(column, direction));
            assert_eq!(
                walked,
                expected_order(&tasks, sort, &names),
                "{column:?} {direction:?}"
            );
        }
    }
}

#[test]
fn an_empty_due_date_is_last_in_both_directions() {
    let dir = TempDir::new().unwrap();
    let store = store_at(&dir);
    let project = project(&store);
    seed(&store, &project, 40);
    let tasks = TaskRepository::list_for_project(&store, project.id()).unwrap();
    let undated: Vec<TaskId> = tasks
        .iter()
        .filter(|task| task.due_on().is_none())
        .map(Task::id)
        .collect();
    assert!(!undated.is_empty(), "the seed must include empty due dates");

    for direction in [SortDirection::Ascending, SortDirection::Descending] {
        let walked = walk(
            &store,
            project.id(),
            &sorted_query(SortColumn::DueDate, direction),
        );
        let first_empty = walked
            .iter()
            .position(|id| undated.contains(id))
            .expect("some task has no due date");
        assert!(
            walked[first_empty..].iter().all(|id| undated.contains(id)),
            "{direction:?}: every undated task comes after every dated one"
        );
        assert_eq!(walked.len() - first_empty, undated.len());
    }
}

#[test]
fn the_sort_keeps_the_filters_and_the_search() {
    let dir = TempDir::new().unwrap();
    let store = store_at(&dir);
    let project = project(&store);
    let names = seed(&store, &project, 137);
    let tasks = TaskRepository::list_for_project(&store, project.id()).unwrap();

    let mut filter = TaskFilter {
        tag_match: TagMatch::Any,
        ..Default::default()
    };
    filter.toggle_priority(Some(Priority::High));
    filter.toggle_priority(None);
    filter.toggle_tag("web");
    let sort = ListSort {
        column: SortColumn::Estimate,
        direction: SortDirection::Descending,
    };
    let query = TaskQuery::new(ALL_STATUSES.to_vec(), filter, "title 1").with_sort(sort);

    let matching: Vec<Task> = tasks
        .iter()
        .filter(|task| query.matches(task))
        .cloned()
        .collect();
    assert!(!matching.is_empty());
    assert_eq!(
        walk(&store, project.id(), &query),
        expected_order(&matching, sort, &names),
        "the sorted, filtered, searched list is the matching tasks in order"
    );
    let total = store
        .list_page(project.id(), &query, None, PAGE_SIZE)
        .unwrap()
        .total;
    assert_eq!(
        total,
        matching.len() as u64,
        "the count follows the same filters"
    );
}

#[test]
fn a_run_of_tasks_with_one_value_is_walked_once_each() {
    let dir = TempDir::new().unwrap();
    let store = store_at(&dir);
    let project = project(&store);
    seed(&store, &project, 60);
    // Tasks 0 to 9 share the due date 2026-10-05, so a sort on it has long ties.
    let walked = walk(
        &store,
        project.id(),
        &sorted_query(SortColumn::DueDate, SortDirection::Ascending),
    );
    let unique: std::collections::HashSet<TaskId> = walked.iter().copied().collect();
    assert_eq!(
        unique.len(),
        walked.len(),
        "no task shows twice across pages"
    );
    assert_eq!(walked.len(), 60);
}

#[test]
fn an_edit_during_a_sorted_walk_moves_only_the_edited_row() {
    let dir = TempDir::new().unwrap();
    let store = store_at(&dir);
    let project = project(&store);
    seed(&store, &project, 137);
    let query = sorted_query(SortColumn::Title, SortDirection::Ascending);
    let before = walk(&store, project.id(), &query);

    // Change the title of a task that comes late. The walk continues from a value, so the rest of the
    // rows keep their place. Only the edited task changes position in the order.
    let mut tasks = TaskRepository::list_for_project(&store, project.id()).unwrap();
    let edited_id = before[120];
    let task = tasks
        .iter_mut()
        .find(|task| task.id() == edited_id)
        .unwrap();
    task.rename("AAAA first").unwrap();
    TaskRepository::save(&store, task).unwrap();

    let after = walk(&store, project.id(), &query);
    let mut without_edited: Vec<TaskId> = after.clone();
    without_edited.retain(|id| *id != edited_id);
    let mut expected: Vec<TaskId> = before.clone();
    expected.retain(|id| *id != edited_id);
    assert_eq!(without_edited, expected, "the other rows keep their order");
    assert_eq!(after.len(), before.len(), "nothing is lost or repeated");
    let unique: std::collections::HashSet<TaskId> = after.iter().copied().collect();
    assert_eq!(unique.len(), after.len());
}

#[test]
fn the_list_settings_round_trip_and_default_when_never_saved() {
    let dir = TempDir::new().unwrap();
    let store = store_at(&dir);
    let project = project(&store);
    assert_eq!(
        store.load_list_view(project.id()).unwrap(),
        ListView::default()
    );

    let mut view = ListView::default();
    view.sort_by(ListColumn::DueDate);
    view.sort_by(ListColumn::DueDate);
    view.toggle_column(ListColumn::Tags);
    store.save_list_view(project.id(), &view).unwrap();

    assert_eq!(store.load_list_view(project.id()).unwrap(), view);
}

#[test]
fn saving_again_replaces_the_settings() {
    let dir = TempDir::new().unwrap();
    let store = store_at(&dir);
    let project = project(&store);
    let mut view = ListView::default();
    view.sort_by(ListColumn::Title);
    store.save_list_view(project.id(), &view).unwrap();
    view.sort_by(ListColumn::Estimate);
    store.save_list_view(project.id(), &view).unwrap();

    let loaded = store.load_list_view(project.id()).unwrap();
    assert_eq!(loaded.sort.column, SortColumn::Estimate);
}

#[test]
fn the_sort_order_and_filter_survive_closing_and_reopening_the_database() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("looji.db");
    let store = SqliteStore::open(&path).unwrap();
    let project = project(&store);
    let names = seed(&store, &project, 137);
    let tasks = TaskRepository::list_for_project(&store, project.id()).unwrap();

    let mut filter = TaskFilter::default();
    filter.toggle_status(Status::Todo);
    filter.toggle_status(Status::Done);
    let mut view = ListView::default();
    view.sort_by(ListColumn::Priority);
    view.sort_by(ListColumn::Priority);
    view.toggle_column(ListColumn::Estimate);
    store.save_list_view(project.id(), &view).unwrap();
    store.save_filter(project.id(), &filter).unwrap();
    let sort = view.sort;
    let before = walk(
        &store,
        project.id(),
        &TaskQuery::new(ALL_STATUSES.to_vec(), filter.clone(), "").with_sort(sort),
    );
    drop(store);

    let reopened = SqliteStore::open(&path).unwrap();
    assert_eq!(reopened.load_list_view(project.id()).unwrap(), view);
    let loaded_filter = reopened.load_filter(project.id()).unwrap();
    let after = walk(
        &reopened,
        project.id(),
        &TaskQuery::new(ALL_STATUSES.to_vec(), loaded_filter, "").with_sort(sort),
    );
    assert_eq!(
        after, before,
        "the same rows, in the same order, after a restart"
    );
    let matching: Vec<Task> = tasks
        .iter()
        .filter(|task| filter.matches(task))
        .cloned()
        .collect();
    assert_eq!(after, expected_order(&matching, sort, &names));
}

#[test]
fn stored_values_that_no_longer_parse_fall_back_to_defaults() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("looji.db");
    let store = SqliteStore::open(&path).unwrap();
    let project = project(&store);
    drop(store);
    let connection = open_database(&path).unwrap();
    connection
        .execute(
            "INSERT INTO project_list_view (project_id, sort_column, sort_direction, visible_columns)
             VALUES (?1, 'retired_column', 'desc', '[\"title\",\"retired\"]')",
            [project.id().as_uuid().to_string()],
        )
        .unwrap();
    drop(connection);

    let store = SqliteStore::open(&path).unwrap();
    let loaded = store.load_list_view(project.id()).unwrap();

    assert_eq!(
        loaded.sort.column,
        SortColumn::Manual,
        "an unknown sort is dropped"
    );
    assert_eq!(
        loaded.sort.direction,
        SortDirection::Descending,
        "the direction still reads"
    );
    assert_eq!(loaded.columns, [ListColumn::Title]);
}

#[test]
fn unreadable_column_lists_fall_back_to_every_column() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("looji.db");
    let store = SqliteStore::open(&path).unwrap();
    let project = project(&store);
    drop(store);
    let connection = open_database(&path).unwrap();
    connection
        .execute(
            "INSERT INTO project_list_view (project_id, sort_column, sort_direction, visible_columns)
             VALUES (?1, 'manual', 'asc', 'not json')",
            [project.id().as_uuid().to_string()],
        )
        .unwrap();
    drop(connection);

    let store = SqliteStore::open(&path).unwrap();
    assert_eq!(
        store.load_list_view(project.id()).unwrap().columns,
        ListColumn::ALL
    );
}

#[test]
fn a_direction_outside_asc_and_desc_is_refused_by_the_database() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("looji.db");
    let store = SqliteStore::open(&path).unwrap();
    let project = project(&store);
    drop(store);
    let connection = open_database(&path).unwrap();

    let inserted = connection.execute(
        "INSERT INTO project_list_view (project_id, sort_column, sort_direction, visible_columns)
         VALUES (?1, 'manual', 'sideways', '[]')",
        [project.id().as_uuid().to_string()],
    );
    assert!(inserted.is_err());
}

#[test]
fn deleting_a_project_removes_its_list_settings() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("looji.db");
    let store = SqliteStore::open(&path).unwrap();
    let project = project(&store);
    let mut view = ListView::default();
    view.sort_by(ListColumn::Title);
    store.save_list_view(project.id(), &view).unwrap();

    looji_domain::ProjectRepository::delete(&store, project.id()).unwrap();

    let connection = open_database(&path).unwrap();
    let rows: i64 = connection
        .query_row("SELECT COUNT(*) FROM project_list_view", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(rows, 0);
}

#[test]
fn a_saved_sort_value_is_a_text_key_for_text_fields_and_a_number_for_numbers() {
    // The cursor carries the sort value, so its type must match what the sort compares.
    let dir = TempDir::new().unwrap();
    let store = store_at(&dir);
    let project = project(&store);
    seed(&store, &project, 20);
    let query = sorted_query(SortColumn::Estimate, SortDirection::Ascending);
    let first = store.list_page(project.id(), &query, None, 5).unwrap();
    match first.next.unwrap().value {
        Some(SortValue::Integer(_)) | None => {}
        other => panic!("estimate cursors are numbers or empty, got {other:?}"),
    }
}
