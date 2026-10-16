use looji_domain::{
    DueDate, Position, Project, ProjectRepository, Status, Task, TaskEventKind, TaskMove,
    TaskRepository, Timestamp, Workspace, WorkspaceRepository, needs_estimate_hint,
};
use looji_persistence::SqliteStore;
use tempfile::TempDir;

fn day(year: i32, month: u32, day: u32) -> DueDate {
    DueDate::new(year, month, day).unwrap()
}

/// A moment in UTC, as the app sees "now" for a move.
fn at(day: DueDate, hour: u64, minute: u64) -> Timestamp {
    let millis = day.day_number() as u64 * 86_400_000 + (hour * 60 + minute) * 60_000;
    Timestamp::from_unix_millis(millis as i64)
}

fn project(store: &SqliteStore) -> Project {
    let workspace = Workspace::new("Work").unwrap();
    WorkspaceRepository::save(store, &workspace).unwrap();
    let project = Project::new(workspace.id(), "looji").unwrap();
    ProjectRepository::save(store, &project).unwrap();
    project
}

/// A task in To do, with the given estimate in minutes (or none).
fn task(store: &SqliteStore, project: &Project, minutes: Option<u32>) -> Task {
    let mut task = Task::new(
        project.id(),
        "Write copy",
        Status::Todo,
        Position::new("a0").unwrap(),
    )
    .unwrap();
    task.set_estimate_minutes(minutes.map(i64::from)).unwrap();
    TaskRepository::save(store, &task).unwrap();
    task
}

fn move_task(store: &SqliteStore, task: &Task, from: Status, to: Status, when: Timestamp) {
    let mv = TaskMove::new(task.id(), task.project_id(), from, to, 0, &Status::ALL).unwrap();
    store.move_task(&mv, when).unwrap();
}

fn due_of(store: &SqliteStore, task: &Task) -> Option<DueDate> {
    TaskRepository::find(store, task.id())
        .unwrap()
        .unwrap()
        .due_on()
}

#[test]
fn the_first_move_into_progress_sets_the_due_date_from_the_estimate() {
    let dir = TempDir::new().unwrap();
    let store = SqliteStore::open(&dir.path().join("looji.db")).unwrap();
    let project = project(&store);
    let task = task(&store, &project, Some(240));

    // Monday 12 October 2026 at 09:00, four working hours: due the same day.
    move_task(
        &store,
        &task,
        Status::Todo,
        Status::InProgress,
        at(day(2026, 10, 12), 9, 0),
    );

    assert_eq!(due_of(&store, &task), Some(day(2026, 10, 12)));
}

#[test]
fn a_start_late_in_the_afternoon_is_due_on_the_next_workday() {
    let dir = TempDir::new().unwrap();
    let store = SqliteStore::open(&dir.path().join("looji.db")).unwrap();
    let project = project(&store);
    let task = task(&store, &project, Some(240));

    // Monday at 16:00: one hour is left that day, and three more fall on Tuesday.
    move_task(
        &store,
        &task,
        Status::Todo,
        Status::InProgress,
        at(day(2026, 10, 12), 16, 0),
    );

    assert_eq!(due_of(&store, &task), Some(day(2026, 10, 13)));
}

#[test]
fn a_weekend_start_counts_from_monday_morning() {
    let dir = TempDir::new().unwrap();
    let store = SqliteStore::open(&dir.path().join("looji.db")).unwrap();
    let project = project(&store);
    let task = task(&store, &project, Some(240));

    // Saturday 17 October: the four hours run from Monday 19 October at 09:00, so they end that day.
    move_task(
        &store,
        &task,
        Status::Todo,
        Status::InProgress,
        at(day(2026, 10, 17), 10, 0),
    );

    assert_eq!(due_of(&store, &task), Some(day(2026, 10, 19)));
}

#[test]
fn moving_back_and_forth_into_progress_never_changes_an_existing_date() {
    let dir = TempDir::new().unwrap();
    let store = SqliteStore::open(&dir.path().join("looji.db")).unwrap();
    let project = project(&store);
    let task = task(&store, &project, Some(240));
    move_task(
        &store,
        &task,
        Status::Todo,
        Status::InProgress,
        at(day(2026, 10, 12), 9, 0),
    );
    let first = due_of(&store, &task);

    // Out to In review and back, at a later time that would give a different date if it were recalculated.
    move_task(
        &store,
        &task,
        Status::InProgress,
        Status::InReview,
        at(day(2026, 10, 13), 9, 0),
    );
    move_task(
        &store,
        &task,
        Status::InReview,
        Status::InProgress,
        at(day(2026, 10, 14), 15, 0),
    );

    assert_eq!(first, Some(day(2026, 10, 12)));
    assert_eq!(due_of(&store, &task), first);
}

#[test]
fn going_back_to_backlog_and_into_progress_again_keeps_the_date() {
    let dir = TempDir::new().unwrap();
    let store = SqliteStore::open(&dir.path().join("looji.db")).unwrap();
    let project = project(&store);
    let task = task(&store, &project, Some(240));
    move_task(
        &store,
        &task,
        Status::Todo,
        Status::InProgress,
        at(day(2026, 10, 12), 9, 0),
    );
    move_task(
        &store,
        &task,
        Status::InProgress,
        Status::Backlog,
        at(day(2026, 10, 13), 9, 0),
    );

    move_task(
        &store,
        &task,
        Status::Backlog,
        Status::InProgress,
        at(day(2026, 10, 15), 11, 0),
    );

    assert_eq!(due_of(&store, &task), Some(day(2026, 10, 12)));
}

#[test]
fn a_task_without_an_estimate_stays_unscheduled_and_gets_the_hint() {
    let dir = TempDir::new().unwrap();
    let store = SqliteStore::open(&dir.path().join("looji.db")).unwrap();
    let project = project(&store);
    let task = task(&store, &project, None);

    move_task(
        &store,
        &task,
        Status::Todo,
        Status::InProgress,
        at(day(2026, 10, 12), 9, 0),
    );

    let stored = TaskRepository::find(&store, task.id()).unwrap().unwrap();
    assert_eq!(stored.due_on(), None);
    assert!(needs_estimate_hint(stored.status(), stored.due_on(), None));
}

#[test]
fn a_zero_estimate_is_treated_as_none_and_never_gives_a_due_date_equal_to_the_start() {
    let dir = TempDir::new().unwrap();
    let store = SqliteStore::open(&dir.path().join("looji.db")).unwrap();
    let project = project(&store);
    let task = task(&store, &project, Some(0));

    move_task(
        &store,
        &task,
        Status::Todo,
        Status::InProgress,
        at(day(2026, 10, 12), 9, 0),
    );

    assert_eq!(due_of(&store, &task), None);
    let stored = TaskRepository::find(&store, task.id()).unwrap().unwrap();
    assert!(needs_estimate_hint(stored.status(), None, Some(0)));
}

#[test]
fn a_reorder_within_progress_does_not_schedule_a_task_that_has_no_date() {
    let dir = TempDir::new().unwrap();
    let store = SqliteStore::open(&dir.path().join("looji.db")).unwrap();
    let project = project(&store);
    let task = task(&store, &project, Some(240));
    let mut in_progress = TaskRepository::find(&store, task.id()).unwrap().unwrap();
    in_progress.set_status(Status::InProgress);
    TaskRepository::save(&store, &in_progress).unwrap();

    move_task(
        &store,
        &in_progress,
        Status::InProgress,
        Status::InProgress,
        at(day(2026, 10, 12), 9, 0),
    );

    assert_eq!(
        due_of(&store, &task),
        None,
        "only a move into progress schedules"
    );
}

#[test]
fn an_existing_date_is_never_overwritten_by_a_move() {
    let dir = TempDir::new().unwrap();
    let store = SqliteStore::open(&dir.path().join("looji.db")).unwrap();
    let project = project(&store);
    let mut task = task(&store, &project, Some(240));
    task.set_due_on(Some(day(2026, 11, 1)));
    TaskRepository::save(&store, &task).unwrap();

    move_task(
        &store,
        &task,
        Status::Todo,
        Status::InProgress,
        at(day(2026, 10, 12), 9, 0),
    );

    assert_eq!(due_of(&store, &task), Some(day(2026, 11, 1)));
}

#[test]
fn a_cleared_date_is_set_again_by_the_next_move_into_progress() {
    // The decision this documents: clearing a date by hand doesn't stop the next move into In progress from
    // setting one. A flag to stop that would need its own migration.
    let dir = TempDir::new().unwrap();
    let store = SqliteStore::open(&dir.path().join("looji.db")).unwrap();
    let project = project(&store);
    let task = task(&store, &project, Some(240));
    move_task(
        &store,
        &task,
        Status::Todo,
        Status::InProgress,
        at(day(2026, 10, 12), 9, 0),
    );
    let mut cleared = TaskRepository::find(&store, task.id()).unwrap().unwrap();
    cleared.set_due_on(None);
    TaskRepository::save(&store, &cleared).unwrap();
    move_task(
        &store,
        &cleared,
        Status::InProgress,
        Status::Todo,
        at(day(2026, 10, 13), 9, 0),
    );

    move_task(
        &store,
        &cleared,
        Status::Todo,
        Status::InProgress,
        at(day(2026, 10, 14), 9, 0),
    );

    assert_eq!(due_of(&store, &task), Some(day(2026, 10, 14)));
}

#[test]
fn the_date_it_sets_is_recorded_in_the_history_as_an_edit_to_the_due_date() {
    let dir = TempDir::new().unwrap();
    let store = SqliteStore::open(&dir.path().join("looji.db")).unwrap();
    let project = project(&store);
    let task = task(&store, &project, Some(240));

    move_task(
        &store,
        &task,
        Status::Todo,
        Status::InProgress,
        at(day(2026, 10, 12), 9, 0),
    );

    let history = store.task_history(task.id()).unwrap();
    assert!(
        history.iter().any(|event| event.kind
            == TaskEventKind::Edited {
                fields: vec!["due".to_string()]
            }),
        "{history:?}"
    );
}

#[test]
fn the_hint_shows_only_for_a_task_in_progress_without_a_date_or_an_estimate() {
    assert!(needs_estimate_hint(Status::InProgress, None, None));
    assert!(needs_estimate_hint(Status::InProgress, None, Some(0)));
    assert!(!needs_estimate_hint(Status::InProgress, None, Some(60)));
    assert!(!needs_estimate_hint(
        Status::InProgress,
        Some(day(2026, 10, 12)),
        None
    ));
    assert!(!needs_estimate_hint(Status::Todo, None, None));
}
