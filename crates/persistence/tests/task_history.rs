use looji_domain::{
    Position, Project, ProjectId, ProjectRepository, Status, Task, TaskEventKind, TaskId, TaskMove,
    TaskRepository, Timestamp, Workspace, WorkspaceRepository,
};
use looji_persistence::SqliteStore;

fn project(store: &SqliteStore) -> Project {
    let workspace = Workspace::new("Work").unwrap();
    WorkspaceRepository::save(store, &workspace).unwrap();
    let project = Project::new(workspace.id(), "Website").unwrap();
    ProjectRepository::save(store, &project).unwrap();
    project
}

fn create(store: &SqliteStore, project: ProjectId) -> Task {
    let task = Task::new(
        project,
        "Write copy",
        Status::Todo,
        Position::new(format!("a{}", uuid::Uuid::new_v4().simple())).unwrap(),
    )
    .unwrap();
    store
        .save_task_recording(&task, &TaskEventKind::Created)
        .unwrap();
    task
}

#[test]
fn creating_a_task_records_one_created_event() {
    let store = SqliteStore::open_in_memory().unwrap();
    let website = project(&store);
    let task = create(&store, website.id());

    let history = store.task_history(task.id()).unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].kind, TaskEventKind::Created);
}

#[test]
fn an_edit_records_the_fields_that_changed() {
    let store = SqliteStore::open_in_memory().unwrap();
    let website = project(&store);
    let mut task = create(&store, website.id());

    task.rename("Write more copy").unwrap();
    store
        .save_task_recording(
            &task,
            &TaskEventKind::Edited {
                fields: vec!["title".to_string()],
            },
        )
        .unwrap();

    let history = store.task_history(task.id()).unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(
        history[1].kind,
        TaskEventKind::Edited {
            fields: vec!["title".to_string()]
        }
    );
    let stored = TaskRepository::find(&store, task.id()).unwrap().unwrap();
    assert_eq!(stored.title(), "Write more copy");
}

#[test]
fn status_moves_and_edits_appear_in_one_history_oldest_first() {
    let store = SqliteStore::open_in_memory().unwrap();
    let website = project(&store);
    let mut task = create(&store, website.id());
    task.rename("Write more copy").unwrap();
    store
        .save_task_recording(
            &task,
            &TaskEventKind::Edited {
                fields: vec!["title".to_string()],
            },
        )
        .unwrap();
    let mv = TaskMove::new(
        task.id(),
        website.id(),
        Status::Todo,
        Status::Done,
        0,
        &Status::DEFAULTS,
    )
    .unwrap();
    store.move_task(&mv, Timestamp::now()).unwrap();

    let kinds: Vec<TaskEventKind> = store
        .task_history(task.id())
        .unwrap()
        .into_iter()
        .map(|event| event.kind)
        .collect();
    assert_eq!(kinds.len(), 3);
    assert_eq!(kinds[0], TaskEventKind::Created);
    assert_eq!(
        kinds[2],
        TaskEventKind::StatusChanged {
            from: Status::Todo,
            to: Status::Done
        }
    );
}

#[test]
fn a_status_change_cannot_be_recorded_as_an_edit() {
    let store = SqliteStore::open_in_memory().unwrap();
    let website = project(&store);
    let task = create(&store, website.id());
    let result = store.save_task_recording(
        &task,
        &TaskEventKind::StatusChanged {
            from: Status::Todo,
            to: Status::Done,
        },
    );
    assert!(result.is_err());
}

#[test]
fn deleting_a_task_removes_its_history() {
    let store = SqliteStore::open_in_memory().unwrap();
    let website = project(&store);
    let task = create(&store, website.id());
    TaskRepository::delete(&store, task.id()).unwrap();

    assert!(store.task_history(task.id()).unwrap().is_empty());
}

#[test]
fn history_is_kept_per_task() {
    let store = SqliteStore::open_in_memory().unwrap();
    let website = project(&store);
    let first = create(&store, website.id());
    let second = create(&store, website.id());

    assert_eq!(store.task_history(first.id()).unwrap().len(), 1);
    assert_eq!(store.task_history(second.id()).unwrap().len(), 1);
    assert_eq!(
        store.task_history(TaskId::new()).unwrap().len(),
        0,
        "an unknown task has no history"
    );
}
