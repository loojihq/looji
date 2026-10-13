use looji_domain::{
    ActivityKind, DomainError, Position, Project, ProjectId, ProjectRepository, Status, Task,
    TaskId, TaskMove, TaskRepository, Timestamp, Workspace, WorkspaceRepository,
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

fn create(store: &SqliteStore, project: ProjectId, status: Status) -> Task {
    let task = Task::new(project, "Write copy", status, Position::new("a0").unwrap()).unwrap();
    TaskRepository::save(store, &task).unwrap();
    task
}

fn status_of(store: &SqliteStore, task: TaskId) -> Status {
    TaskRepository::find(store, task).unwrap().unwrap().status()
}

fn move_of(task: &Task, to: Status) -> TaskMove {
    TaskMove::new(
        task.id(),
        task.project_id(),
        task.status(),
        to,
        &Status::DEFAULTS,
    )
    .unwrap()
}

#[test]
fn a_move_writes_the_status_and_one_history_entry() {
    let store = SqliteStore::open_in_memory().unwrap();
    let website = project(&store);
    let task = create(&store, website.id(), Status::Todo);
    let at = Timestamp::now();

    let activity = store
        .move_task(&move_of(&task, Status::InProgress), at)
        .unwrap();

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
fn the_history_keeps_every_move_in_order() {
    let store = SqliteStore::open_in_memory().unwrap();
    let website = project(&store);
    let mut task = create(&store, website.id(), Status::Todo);

    store
        .move_task(&move_of(&task, Status::InProgress), Timestamp::now())
        .unwrap();
    task = TaskRepository::find(&store, task.id()).unwrap().unwrap();
    store
        .move_task(&move_of(&task, Status::Done), Timestamp::now())
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
fn a_failed_history_write_leaves_the_status_unchanged() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("looji.db");
    let store = SqliteStore::open(&path).unwrap();
    let website = project(&store);
    let task = create(&store, website.id(), Status::Todo);

    // The status update runs first. This makes the history insert fail, so the update must roll back.
    let raw = Connection::open(&path).unwrap();
    raw.execute_batch(
        "CREATE TRIGGER block_history BEFORE INSERT ON task_activity
         BEGIN SELECT RAISE(ABORT, 'forced failure'); END;",
    )
    .unwrap();
    drop(raw);

    let result = store.move_task(&move_of(&task, Status::Done), Timestamp::now());

    assert!(matches!(result, Err(MoveError::Storage(_))), "{result:?}");
    assert_eq!(status_of(&store, task.id()), Status::Todo);
    assert!(store.task_activity(task.id()).unwrap().is_empty());
    drop(store);
}

#[test]
fn a_task_that_moved_elsewhere_is_a_conflict_and_nothing_is_written() {
    let store = SqliteStore::open_in_memory().unwrap();
    let website = project(&store);
    let task = create(&store, website.id(), Status::Todo);
    // The move was built from `Todo`, but the task has moved to `Done` since.
    let mv = move_of(&task, Status::InProgress);
    let mut moved = TaskRepository::find(&store, task.id()).unwrap().unwrap();
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
    let task = create(&store, website.id(), Status::Todo);
    TaskRepository::delete(&store, task.id()).unwrap();

    let result = store.move_task(&move_of(&task, Status::Done), Timestamp::now());

    assert!(matches!(result, Err(MoveError::NotFound)), "{result:?}");
}

#[test]
fn moving_into_a_hidden_column_or_the_same_column_is_rejected_before_any_write() {
    let task = TaskId::new();
    let project = ProjectId::new();
    let hidden = TaskMove::new(
        task,
        project,
        Status::Todo,
        Status::InReview,
        &[Status::Todo, Status::Done],
    );
    assert!(matches!(hidden, Err(DomainError::StatusNotVisible { .. })));

    let same = TaskMove::new(task, project, Status::Todo, Status::Todo, &Status::DEFAULTS);
    assert_eq!(same, Err(DomainError::SameStatus));
}

#[test]
fn a_move_survives_closing_and_reopening_the_database() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("looji.db");
    let (task_id, project_id) = {
        let store = SqliteStore::open(&path).unwrap();
        let website = project(&store);
        let task = create(&store, website.id(), Status::Todo);
        store
            .move_task(&move_of(&task, Status::InProgress), Timestamp::now())
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
