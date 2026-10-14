use looji_domain::{
    DueDate, Position, Priority, Project, ProjectId, ProjectRepository, Status, Task,
    TaskRepository, Workspace, WorkspaceRepository, normalize_tags,
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
        Position::new("a0").unwrap(),
    )
    .unwrap();
    TaskRepository::save(store, &task).unwrap();
    task
}

#[test]
fn priority_due_date_and_tags_survive_a_save_and_a_reload() {
    let store = SqliteStore::open_in_memory().unwrap();
    let website = project(&store);
    let mut task = create(&store, website.id());

    task.set_priority(Some(Priority::High));
    task.set_due_on(DueDate::new(2026, 10, 20));
    task.set_tags(normalize_tags(["web", "Copy"]).unwrap());
    TaskRepository::save(&store, &task).unwrap();

    let loaded = TaskRepository::find(&store, task.id()).unwrap().unwrap();
    assert_eq!(loaded.priority(), Some(Priority::High));
    assert_eq!(loaded.due_on(), DueDate::new(2026, 10, 20));
    assert_eq!(loaded.tags(), ["copy", "web"]);

    let listed = TaskRepository::list_for_project(&store, website.id()).unwrap();
    assert_eq!(listed[0].tags(), ["copy", "web"]);
    assert_eq!(listed[0].priority(), Some(Priority::High));
}

#[test]
fn saving_replaces_the_tags_rather_than_adding_to_them() {
    let store = SqliteStore::open_in_memory().unwrap();
    let website = project(&store);
    let mut task = create(&store, website.id());
    task.set_tags(normalize_tags(["a", "b"]).unwrap());
    TaskRepository::save(&store, &task).unwrap();

    task.set_tags(normalize_tags(["c"]).unwrap());
    TaskRepository::save(&store, &task).unwrap();

    let loaded = TaskRepository::find(&store, task.id()).unwrap().unwrap();
    assert_eq!(loaded.tags(), ["c"]);
}

#[test]
fn clearing_the_optional_fields_clears_them_in_storage() {
    let store = SqliteStore::open_in_memory().unwrap();
    let website = project(&store);
    let mut task = create(&store, website.id());
    task.set_priority(Some(Priority::Low));
    task.set_due_on(DueDate::new(2026, 1, 1));
    task.set_tags(normalize_tags(["x"]).unwrap());
    TaskRepository::save(&store, &task).unwrap();

    task.set_priority(None);
    task.set_due_on(None);
    task.set_tags(Vec::new());
    TaskRepository::save(&store, &task).unwrap();

    let loaded = TaskRepository::find(&store, task.id()).unwrap().unwrap();
    assert_eq!(loaded.priority(), None);
    assert_eq!(loaded.due_on(), None);
    assert!(loaded.tags().is_empty());
}

#[test]
fn the_database_rejects_a_priority_outside_the_known_values() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("looji.db");
    let store = SqliteStore::open(&path).unwrap();
    let website = project(&store);
    let task = create(&store, website.id());
    let raw = looji_persistence::Connection::open(&path).unwrap();
    let result = raw.execute(
        "UPDATE tasks SET priority = 'urgent' WHERE id = ?1",
        [task.id().as_uuid().to_string()],
    );
    assert!(result.is_err());
    drop(raw);
    drop(store);
}
