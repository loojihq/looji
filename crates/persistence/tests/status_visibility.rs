use looji_domain::{
    DomainError, Position, Project, ProjectId, ProjectRepository, Status, Task, TaskRepository,
    Workspace, WorkspaceRepository,
};
use looji_persistence::{Connection, SqliteStore, VisibilityError, open_database};
use tempfile::TempDir;

fn project(store: &SqliteStore) -> Project {
    let workspace = Workspace::new("Work").unwrap();
    WorkspaceRepository::save(store, &workspace).unwrap();
    let project = Project::new(workspace.id(), "Website").unwrap();
    ProjectRepository::save(store, &project).unwrap();
    project
}

fn create(store: &SqliteStore, project: ProjectId, status: Status, key: &str) -> Task {
    let task = Task::new(project, "Task", status, Position::new(key).unwrap()).unwrap();
    TaskRepository::save(store, &task).unwrap();
    task
}

/// The number of tasks with each status in the project, in `Status::ALL` order.
fn counts(store: &SqliteStore, project: ProjectId) -> Vec<usize> {
    let tasks = TaskRepository::list_for_project(store, project).unwrap();
    Status::ALL
        .into_iter()
        .map(|status| tasks.iter().filter(|task| task.status() == status).count())
        .collect()
}

fn shown(store: &SqliteStore, project: &Project) -> bool {
    ProjectRepository::find(store, project.workspace_id(), project.id())
        .unwrap()
        .unwrap()
        .show_in_review()
}

#[test]
fn hiding_moves_tasks_to_the_nearest_visible_status_and_showing_restores_them() {
    let store = SqliteStore::open_in_memory().unwrap();
    let website = project(&store);
    create(&store, website.id(), Status::InReview, "a0");
    create(&store, website.id(), Status::InReview, "a1");
    create(&store, website.id(), Status::InProgress, "a2");
    // Backlog, Todo, InProgress, InReview, Done
    assert_eq!(counts(&store, website.id()), vec![0, 0, 1, 2, 0]);

    let moved = store
        .set_status_visible(website.id(), Status::InReview, false)
        .unwrap();

    // The tie between In progress and Done goes to In progress.
    assert_eq!(moved, 2);
    assert_eq!(counts(&store, website.id()), vec![0, 0, 3, 0, 0]);
    assert!(!shown(&store, &website));

    let restored = store
        .set_status_visible(website.id(), Status::InReview, true)
        .unwrap();

    assert_eq!(restored, 2);
    assert_eq!(counts(&store, website.id()), vec![0, 0, 1, 2, 0]);
    assert!(shown(&store, &website));
}

#[test]
fn the_four_defaults_cannot_be_hidden_in_the_repository() {
    let store = SqliteStore::open_in_memory().unwrap();
    let website = project(&store);

    for status in Status::DEFAULTS {
        let result = store.set_status_visible(website.id(), status, false);
        assert!(
            matches!(
                result,
                Err(VisibilityError::Domain(
                    DomainError::StatusNotHideable { .. }
                ))
            ),
            "{status:?} must not be hideable, got {result:?}"
        );
    }
    assert!(shown(&store, &website));
}

#[test]
fn a_task_the_user_moved_while_hidden_is_not_restored() {
    let store = SqliteStore::open_in_memory().unwrap();
    let website = project(&store);
    let kept = create(&store, website.id(), Status::InReview, "a0");
    let moved_on = create(&store, website.id(), Status::InReview, "a1");
    store
        .set_status_visible(website.id(), Status::InReview, false)
        .unwrap();

    // The user moves one of the moved tasks to Done while the column is hidden.
    let mut changed = TaskRepository::find(&store, moved_on.id())
        .unwrap()
        .unwrap();
    changed.set_status(Status::Done);
    TaskRepository::save(&store, &changed).unwrap();

    let restored = store
        .set_status_visible(website.id(), Status::InReview, true)
        .unwrap();

    assert_eq!(
        restored, 1,
        "only the task still at the fallback comes back"
    );
    let kept_now = TaskRepository::find(&store, kept.id()).unwrap().unwrap();
    assert_eq!(kept_now.status(), Status::InReview);
    let changed_now = TaskRepository::find(&store, moved_on.id())
        .unwrap()
        .unwrap();
    assert_eq!(
        changed_now.status(),
        Status::Done,
        "the user's move must stand"
    );
}

#[test]
fn a_failed_toggle_leaves_the_tasks_and_the_flag_unchanged() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("looji.db");
    let store = SqliteStore::open(&path).unwrap();
    let website = project(&store);
    create(&store, website.id(), Status::InReview, "a0");
    create(&store, website.id(), Status::InReview, "a1");
    let before = counts(&store, website.id());

    // The task rows are updated first. This makes the project update fail, so the task
    // changes must roll back with it.
    let raw = Connection::open(&path).unwrap();
    raw.execute_batch(
        "CREATE TRIGGER block_flag BEFORE UPDATE OF show_in_review ON projects
         BEGIN SELECT RAISE(ABORT, 'forced failure'); END;",
    )
    .unwrap();
    drop(raw);

    let result = store.set_status_visible(website.id(), Status::InReview, false);

    assert!(
        matches!(result, Err(VisibilityError::Storage(_))),
        "got {result:?}"
    );
    assert_eq!(counts(&store, website.id()), before, "no task may move");
    assert!(shown(&store, &website), "the flag must not change");
    let connection = open_database(&path).unwrap();
    let hidden_markers: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM tasks WHERE hidden_from IS NOT NULL",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(hidden_markers, 0, "no origin markers may be left behind");
}

#[test]
fn the_hidden_flag_survives_a_restart() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("looji.db");
    let (website_id, workspace_id);
    {
        let store = SqliteStore::open(&path).unwrap();
        let website = project(&store);
        website_id = website.id();
        workspace_id = website.workspace_id();
        create(&store, website.id(), Status::InReview, "a0");
        store
            .set_status_visible(website.id(), Status::InReview, false)
            .unwrap();
    }

    let store = SqliteStore::open(&path).unwrap();
    let website = ProjectRepository::find(&store, workspace_id, website_id)
        .unwrap()
        .unwrap();

    assert!(!website.show_in_review());
    assert!(!website.is_status_visible(Status::InReview));
    assert_eq!(counts(&store, website_id), vec![0, 0, 1, 0, 0]);
}

#[test]
fn hiding_one_project_leaves_another_projects_column_alone() {
    let store = SqliteStore::open_in_memory().unwrap();
    let website = project(&store);
    let other = {
        let workspace = Workspace::new("Other").unwrap();
        WorkspaceRepository::save(&store, &workspace).unwrap();
        let project = Project::new(workspace.id(), "API").unwrap();
        ProjectRepository::save(&store, &project).unwrap();
        project
    };
    create(&store, other.id(), Status::InReview, "a0");

    store
        .set_status_visible(website.id(), Status::InReview, false)
        .unwrap();

    assert!(shown(&store, &other));
    assert_eq!(counts(&store, other.id()), vec![0, 0, 0, 1, 0]);
}

#[test]
fn backlog_can_be_hidden_and_shown_like_in_review() {
    let store = SqliteStore::open_in_memory().unwrap();
    let website = project(&store);
    create(&store, website.id(), Status::Backlog, "a0");
    create(&store, website.id(), Status::Todo, "a1");
    // Backlog, Todo, InProgress, InReview, Done
    assert_eq!(counts(&store, website.id()), vec![1, 1, 0, 0, 0]);

    let moved = store
        .set_status_visible(website.id(), Status::Backlog, false)
        .unwrap();

    // The task in Backlog moves to the nearest shown status, To do.
    assert_eq!(moved, 1);
    assert_eq!(counts(&store, website.id()), vec![0, 2, 0, 0, 0]);
    let hidden = ProjectRepository::find(&store, website.workspace_id(), website.id())
        .unwrap()
        .unwrap();
    assert!(!hidden.is_status_visible(Status::Backlog));
    assert!(hidden.is_status_visible(Status::InReview));

    let restored = store
        .set_status_visible(website.id(), Status::Backlog, true)
        .unwrap();

    assert_eq!(restored, 1);
    assert_eq!(counts(&store, website.id()), vec![1, 1, 0, 0, 0]);
}
