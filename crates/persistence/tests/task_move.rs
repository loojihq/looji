use looji_domain::{
    ActivityKind, DomainError, Position, Project, ProjectId, ProjectRepository, Status, Task,
    TaskId, TaskMove, TaskRepository, Timestamp, Workspace, WorkspaceRepository, after,
};
use looji_persistence::{Connection, MoveError, SqliteStore};
use tempfile::TempDir;

fn project(store: &SqliteStore) -> Project {
    let workspace = Workspace::new("Work").unwrap();
    WorkspaceRepository::save(store, &workspace).unwrap();
    let project = Project::new(workspace.id(), "Website").unwrap();
    ProjectRepository::save(store, &project).unwrap();
    project
}

/// Adds a task to the end of `status` in the project, with a key after the column's last key.
fn append(store: &SqliteStore, project: ProjectId, status: Status) -> Task {
    let last = TaskRepository::list_for_project(store, project)
        .unwrap()
        .into_iter()
        .rev()
        .find(|task| task.status() == status)
        .map(|task| task.position().as_str().to_string());
    let key = after(last.as_deref()).unwrap();
    let task = Task::new(project, "Write copy", status, Position::new(key).unwrap()).unwrap();
    TaskRepository::save(store, &task).unwrap();
    task
}

fn find(store: &SqliteStore, task: TaskId) -> Task {
    TaskRepository::find(store, task).unwrap().unwrap()
}

fn status_of(store: &SqliteStore, task: TaskId) -> Status {
    find(store, task).status()
}

/// A move of `task` to `to` at `index` among the column's other tasks, from where it is now.
fn move_of(store: &SqliteStore, task: TaskId, to: Status, index: usize) -> TaskMove {
    let task = find(store, task);
    TaskMove::new(
        task.id(),
        task.project_id(),
        task.status(),
        to,
        index,
        &Status::DEFAULTS,
    )
    .unwrap()
}

#[test]
fn a_status_change_writes_the_status_and_one_history_entry() {
    let store = SqliteStore::open_in_memory().unwrap();
    let website = project(&store);
    let task = append(&store, website.id(), Status::Todo);
    let at = Timestamp::now();

    let activity = store
        .move_task(&move_of(&store, task.id(), Status::InProgress, 0), at)
        .unwrap()
        .expect("a status change writes history");

    assert_eq!(status_of(&store, task.id()), Status::InProgress);
    let history = store.task_activity(task.id()).unwrap();
    assert_eq!(history, [activity]);
    assert_eq!(
        history[0].kind,
        ActivityKind::StatusChanged {
            from: Status::Todo,
            to: Status::InProgress
        }
    );
    assert_eq!(history[0].at, at);
}

#[test]
fn a_reorder_within_a_column_writes_no_history() {
    let store = SqliteStore::open_in_memory().unwrap();
    let website = project(&store);
    let first = append(&store, website.id(), Status::Todo);
    let second = append(&store, website.id(), Status::Todo);

    let activity = store
        .move_task(
            &move_of(&store, second.id(), Status::Todo, 0),
            Timestamp::now(),
        )
        .unwrap();

    assert!(activity.is_none());
    assert!(store.task_activity(second.id()).unwrap().is_empty());
    let order: Vec<TaskId> = TaskRepository::list_for_project(&store, website.id())
        .unwrap()
        .into_iter()
        .map(|task| task.id())
        .collect();
    assert_eq!(order, [second.id(), first.id()]);
}

#[test]
fn the_history_keeps_every_status_change_in_order() {
    let store = SqliteStore::open_in_memory().unwrap();
    let website = project(&store);
    let task = append(&store, website.id(), Status::Todo);

    store
        .move_task(
            &move_of(&store, task.id(), Status::InProgress, 0),
            Timestamp::now(),
        )
        .unwrap();
    store
        .move_task(
            &move_of(&store, task.id(), Status::Done, 0),
            Timestamp::now(),
        )
        .unwrap();

    let kinds: Vec<ActivityKind> = store
        .task_activity(task.id())
        .unwrap()
        .into_iter()
        .map(|activity| activity.kind)
        .collect();
    assert_eq!(
        kinds,
        [
            ActivityKind::StatusChanged {
                from: Status::Todo,
                to: Status::InProgress
            },
            ActivityKind::StatusChanged {
                from: Status::InProgress,
                to: Status::Done
            },
        ]
    );
}

#[test]
fn a_failed_history_write_leaves_the_status_and_the_key_unchanged() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("looji.db");
    let store = SqliteStore::open(&path).unwrap();
    let website = project(&store);
    let task = append(&store, website.id(), Status::Todo);
    let before = find(&store, task.id()).position().as_str().to_string();

    // The status and key update runs first. This makes the history insert fail, so both must roll back.
    let raw = Connection::open(&path).unwrap();
    raw.execute_batch(
        "CREATE TRIGGER block_history BEFORE INSERT ON task_activity
         BEGIN SELECT RAISE(ABORT, 'forced failure'); END;",
    )
    .unwrap();
    drop(raw);

    let result = store.move_task(
        &move_of(&store, task.id(), Status::Done, 0),
        Timestamp::now(),
    );

    assert!(matches!(result, Err(MoveError::Storage(_))), "{result:?}");
    let after_failure = find(&store, task.id());
    assert_eq!(after_failure.status(), Status::Todo);
    assert_eq!(after_failure.position().as_str(), before);
    assert!(store.task_activity(task.id()).unwrap().is_empty());
    drop(store);
}

#[test]
fn a_task_that_moved_elsewhere_is_a_conflict_and_nothing_is_written() {
    let store = SqliteStore::open_in_memory().unwrap();
    let website = project(&store);
    let task = append(&store, website.id(), Status::Todo);
    // The move was built from `Todo`, but the task has moved to `Done` since.
    let mv = move_of(&store, task.id(), Status::InProgress, 0);
    let mut moved = find(&store, task.id());
    moved.set_status(Status::Done);
    TaskRepository::save(&store, &moved).unwrap();

    let result = store.move_task(&mv, Timestamp::now());

    assert!(matches!(result, Err(MoveError::Conflict)), "{result:?}");
    assert_eq!(status_of(&store, task.id()), Status::Done);
    assert!(store.task_activity(task.id()).unwrap().is_empty());
}

#[test]
fn a_missing_task_is_not_found() {
    let store = SqliteStore::open_in_memory().unwrap();
    let website = project(&store);
    let task = append(&store, website.id(), Status::Todo);
    let mv = move_of(&store, task.id(), Status::Done, 0);
    TaskRepository::delete(&store, task.id()).unwrap();

    let result = store.move_task(&mv, Timestamp::now());

    assert!(matches!(result, Err(MoveError::NotFound)), "{result:?}");
}

#[test]
fn moving_into_a_hidden_column_is_rejected_before_any_write() {
    let hidden = TaskMove::new(
        TaskId::new(),
        ProjectId::new(),
        Status::Todo,
        Status::InReview,
        0,
        &[Status::Todo, Status::Done],
    );
    assert!(matches!(hidden, Err(DomainError::StatusNotVisible { .. })));
}

#[test]
fn a_move_survives_closing_and_reopening_the_database() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("looji.db");
    let (task_id, project_id) = {
        let store = SqliteStore::open(&path).unwrap();
        let website = project(&store);
        let task = append(&store, website.id(), Status::Todo);
        store
            .move_task(
                &move_of(&store, task.id(), Status::InProgress, 0),
                Timestamp::now(),
            )
            .unwrap();
        (task.id(), website.id())
    };

    let store = SqliteStore::open(&path).unwrap();
    assert_eq!(status_of(&store, task_id), Status::InProgress);
    assert_eq!(
        TaskRepository::list_for_project(&store, project_id)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(store.task_activity(task_id).unwrap().len(), 1);
    drop(store);
}
