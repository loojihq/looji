use looji_domain::{
    Position, Project, ProjectId, ProjectRepository, Status, Task, TaskRepository, Workspace,
    WorkspaceRepository,
};
use looji_persistence::{SqliteStore, open_database};
use tempfile::TempDir;

fn store() -> SqliteStore {
    SqliteStore::open_in_memory().expect("in-memory store opens and migrates")
}

fn project(store: &SqliteStore, name: &str) -> Project {
    let workspace = Workspace::new("Work").unwrap();
    WorkspaceRepository::save(store, &workspace).unwrap();
    let project = Project::new(workspace.id(), name).unwrap();
    ProjectRepository::save(store, &project).unwrap();
    project
}

/// A task with its own position, since two tasks can't share one in the same column.
fn task(store: &SqliteStore, project: ProjectId, title: &str) -> Task {
    let key = format!("a{}", uuid::Uuid::new_v4().simple());
    let task = Task::new(project, title, Status::Todo, Position::new(key).unwrap()).unwrap();
    TaskRepository::save(store, &task).unwrap();
    task
}

fn suggestions(store: &SqliteStore, project: ProjectId, prefix: &str) -> Vec<String> {
    TaskRepository::tags_in_use(store, project, prefix, 8).unwrap()
}

#[test]
fn adding_the_same_tag_twice_in_different_case_leaves_one_row() {
    let store = store();
    let project = project(&store, "looji");
    let mut task = task(&store, project.id(), "Tagged");
    task.add_tag("Alpha").unwrap();
    task.add_tag("alpha").unwrap();
    TaskRepository::save(&store, &task).unwrap();

    let stored = TaskRepository::find(&store, task.id()).unwrap().unwrap();
    assert_eq!(stored.tags(), ["alpha"]);
}

#[test]
fn the_database_rejects_a_duplicate_tag_that_skips_the_domain() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("looji.db");
    let store = SqliteStore::open(&path).unwrap();
    let project = project(&store, "looji");
    let task = task(&store, project.id(), "Tagged");
    let key = task.id().as_uuid().to_string();
    let connection = open_database(&path).unwrap();

    connection
        .execute(
            "INSERT INTO task_tags (task_id, tag) VALUES (?1, 'alpha')",
            [&key],
        )
        .unwrap();
    let repeat = connection.execute(
        "INSERT INTO task_tags (task_id, tag) VALUES (?1, 'alpha')",
        [&key],
    );
    assert!(repeat.is_err(), "the same tag twice on one task must fail");
}

#[test]
fn the_database_rejects_a_tag_that_is_not_lowercase_or_is_padded() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("looji.db");
    let store = SqliteStore::open(&path).unwrap();
    let project = project(&store, "looji");
    let task = task(&store, project.id(), "Tagged");
    let key = task.id().as_uuid().to_string();
    let connection = open_database(&path).unwrap();

    for raw in ["Alpha", " alpha", "alpha "] {
        let inserted = connection.execute(
            "INSERT INTO task_tags (task_id, tag) VALUES (?1, ?2)",
            [key.as_str(), raw],
        );
        assert!(inserted.is_err(), "{raw:?} is not in stored form");
    }
}

#[test]
fn a_tag_is_suggested_while_a_task_has_it_and_stops_when_the_last_one_drops_it() {
    let store = store();
    let project = project(&store, "looji");
    let mut task = task(&store, project.id(), "Alpha task");
    task.add_tag("alpha").unwrap();
    TaskRepository::save(&store, &task).unwrap();
    assert_eq!(suggestions(&store, project.id(), "al"), ["alpha"]);

    task.remove_tag("alpha");
    TaskRepository::save(&store, &task).unwrap();

    assert!(suggestions(&store, project.id(), "al").is_empty());
}

#[test]
fn a_tag_stays_suggested_while_any_task_still_uses_it() {
    let store = store();
    let project = project(&store, "looji");
    let mut first = task(&store, project.id(), "First");
    let mut second = task(&store, project.id(), "Second");
    first.add_tag("beta").unwrap();
    second.add_tag("beta").unwrap();
    TaskRepository::save(&store, &first).unwrap();
    TaskRepository::save(&store, &second).unwrap();

    first.remove_tag("beta");
    TaskRepository::save(&store, &first).unwrap();
    assert_eq!(suggestions(&store, project.id(), "be"), ["beta"]);

    second.remove_tag("beta");
    TaskRepository::save(&store, &second).unwrap();
    assert!(suggestions(&store, project.id(), "be").is_empty());
}

#[test]
fn suggestions_come_only_from_the_project_and_match_the_prefix_literally() {
    let store = store();
    let workspace = Workspace::new("Work").unwrap();
    WorkspaceRepository::save(&store, &workspace).unwrap();
    let website = Project::new(workspace.id(), "website").unwrap();
    ProjectRepository::save(&store, &website).unwrap();
    let docs = Project::new(workspace.id(), "docs").unwrap();
    ProjectRepository::save(&store, &docs).unwrap();

    let mut here = task(&store, website.id(), "Here");
    here.add_tag("a_b").unwrap();
    here.add_tag("axb").unwrap();
    TaskRepository::save(&store, &here).unwrap();
    let mut there = task(&store, docs.id(), "There");
    there.add_tag("apple").unwrap();
    TaskRepository::save(&store, &there).unwrap();

    assert_eq!(suggestions(&store, website.id(), "a_"), ["a_b"]);
    assert_eq!(suggestions(&store, website.id(), "a"), ["a_b", "axb"]);
    assert!(suggestions(&store, website.id(), "%").is_empty());
    assert_eq!(suggestions(&store, docs.id(), "a"), ["apple"]);
}
