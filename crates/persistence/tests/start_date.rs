use looji_domain::{
    DueDate, Position, Project, ProjectRepository, Status, Task, TaskRepository, Workspace,
    WorkspaceRepository,
};
use looji_persistence::{SqliteStore, open_database};
use tempfile::TempDir;

fn project(store: &SqliteStore) -> Project {
    let workspace = Workspace::new("Work").unwrap();
    WorkspaceRepository::save(store, &workspace).unwrap();
    let project = Project::new(workspace.id(), "looji").unwrap();
    ProjectRepository::save(store, &project).unwrap();
    project
}

fn day(year: i32, month: u32, day: u32) -> DueDate {
    DueDate::new(year, month, day).unwrap()
}

#[test]
fn a_start_and_due_date_round_trip_as_calendar_days() {
    let dir = TempDir::new().unwrap();
    let store = SqliteStore::open(&dir.path().join("looji.db")).unwrap();
    let project = project(&store);
    let mut task = Task::new(
        project.id(),
        "Plan",
        Status::Todo,
        Position::new("a0").unwrap(),
    )
    .unwrap();
    task.set_start_on(Some(day(2028, 2, 29)));
    task.set_due_on(Some(day(2028, 3, 1)));
    TaskRepository::save(&store, &task).unwrap();

    let stored = TaskRepository::find(&store, task.id()).unwrap().unwrap();

    assert_eq!(stored.start_on(), Some(day(2028, 2, 29)));
    assert_eq!(stored.due_on(), Some(day(2028, 3, 1)));
}

#[test]
fn a_start_after_the_due_date_is_kept_as_given_not_swapped() {
    let dir = TempDir::new().unwrap();
    let store = SqliteStore::open(&dir.path().join("looji.db")).unwrap();
    let project = project(&store);
    let mut task = Task::new(
        project.id(),
        "Backwards",
        Status::Todo,
        Position::new("a0").unwrap(),
    )
    .unwrap();
    task.set_start_on(Some(day(2026, 10, 18)));
    task.set_due_on(Some(day(2026, 10, 16)));
    TaskRepository::save(&store, &task).unwrap();

    let stored = TaskRepository::find(&store, task.id()).unwrap().unwrap();

    assert_eq!(stored.start_on(), Some(day(2026, 10, 18)));
    assert_eq!(stored.due_on(), Some(day(2026, 10, 16)));
}

#[test]
fn the_database_refuses_a_start_that_is_not_a_calendar_day() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("looji.db");
    let store = SqliteStore::open(&path).unwrap();
    let project = project(&store);
    let task = Task::new(
        project.id(),
        "Plan",
        Status::Todo,
        Position::new("a0").unwrap(),
    )
    .unwrap();
    TaskRepository::save(&store, &task).unwrap();
    let connection = open_database(&path).unwrap();

    let refused = connection.execute(
        "UPDATE tasks SET start_on = '16/10/2026' WHERE id = ?1",
        [task.id().as_uuid().to_string()],
    );

    assert!(refused.is_err());
}
