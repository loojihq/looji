use looji_domain::{
    Position, Project, ProjectRepository, Status, Task, TaskRepository, Workspace,
    WorkspaceRepository, status_columns,
};
use looji_persistence::SqliteStore;

fn store() -> SqliteStore {
    SqliteStore::open_in_memory().unwrap()
}

fn project(store: &SqliteStore, name: &str) -> Project {
    let workspace = Workspace::new("Work").unwrap();
    WorkspaceRepository::save(store, &workspace).unwrap();
    let project = Project::new(workspace.id(), name).unwrap();
    ProjectRepository::save(store, &project).unwrap();
    project
}

fn create(store: &SqliteStore, project: &Project, status: Status, key: &str) -> Task {
    let task = Task::new(project.id(), "Task", status, Position::new(key).unwrap()).unwrap();
    TaskRepository::save(store, &task).unwrap();
    task
}

/// Column counts in `Status::ALL` order, read from what the board would render.
fn counts(store: &SqliteStore, project: &Project) -> Vec<usize> {
    let tasks = TaskRepository::list_for_project(store, project.id()).unwrap();
    status_columns(&tasks)
        .iter()
        .map(|column| column.count())
        .collect()
}

#[test]
fn tasks_land_in_the_column_for_their_status() {
    let store = store();
    let website = project(&store, "Website");
    create(&store, &website, Status::Backlog, "a0");
    create(&store, &website, Status::Todo, "a1");
    create(&store, &website, Status::Todo, "a2");
    create(&store, &website, Status::Done, "a3");

    // Backlog, Todo, InProgress, Done
    assert_eq!(counts(&store, &website), vec![1, 2, 0, 1]);
}

#[test]
fn counts_update_when_tasks_are_created_and_deleted() {
    let store = store();
    let website = project(&store, "Website");
    let first = create(&store, &website, Status::InProgress, "a0");
    create(&store, &website, Status::InProgress, "a1");
    assert_eq!(counts(&store, &website), vec![0, 0, 2, 0]);

    TaskRepository::delete(&store, first.id()).unwrap();
    assert_eq!(counts(&store, &website), vec![0, 0, 1, 0]);

    create(&store, &website, Status::Done, "a2");
    assert_eq!(counts(&store, &website), vec![0, 0, 1, 1]);
}

#[test]
fn a_board_shows_only_its_own_projects_tasks() {
    let store = store();
    let website = project(&store, "Website");
    let api = {
        let workspace = Workspace::new("Other").unwrap();
        WorkspaceRepository::save(&store, &workspace).unwrap();
        let project = Project::new(workspace.id(), "API").unwrap();
        ProjectRepository::save(&store, &project).unwrap();
        project
    };
    create(&store, &website, Status::Todo, "a0");
    create(&store, &api, Status::Todo, "a0");
    create(&store, &api, Status::Todo, "a1");

    assert_eq!(counts(&store, &website), vec![0, 1, 0, 0]);
    assert_eq!(counts(&store, &api), vec![0, 2, 0, 0]);
}

#[test]
fn an_empty_project_has_four_zero_columns() {
    let store = store();
    let empty = project(&store, "Empty");
    assert_eq!(counts(&store, &empty), vec![0, 0, 0, 0]);
}
