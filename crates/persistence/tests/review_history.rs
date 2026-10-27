//! The reviews a task has had. Each restart is a real close and reopen of the database file.

use looji_domain::{
    Position, Project, ProjectRepository, ReviewHistoryRepository, ReviewRecord, Status, Task,
    TaskRepository, Timestamp, Workspace, WorkspaceRepository,
};
use looji_persistence::{Connection, SqliteStore};
use tempfile::TempDir;

fn record(outcome: &str, at: i64) -> ReviewRecord {
    ReviewRecord {
        outcome: outcome.into(),
        summary: format!("{outcome} summary"),
        review_json: "{}".into(),
        model: Some("claude-sonnet-5-5".into()),
        created_at: Timestamp::from_unix_millis(at),
    }
}

fn task_in(store: &SqliteStore, title: &str) -> Task {
    let workspace = Workspace::new("Work").unwrap();
    WorkspaceRepository::save(store, &workspace).unwrap();
    let project = Project::new(workspace.id(), "looji").unwrap();
    ProjectRepository::save(store, &project).unwrap();
    let task = Task::new(
        project.id(),
        title,
        Status::InReview,
        Position::new("a0").unwrap(),
    )
    .unwrap();
    TaskRepository::save(store, &task).unwrap();
    task
}

#[test]
fn reviews_are_kept_newest_first_across_a_restart() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("looji.db");
    let task = {
        let store = SqliteStore::open(&path).unwrap();
        let task = task_in(&store, "Fix login");
        store
            .add_review(task.id(), &record("needs_work", 100))
            .unwrap();
        store.add_review(task.id(), &record("done", 200)).unwrap();
        task
    };
    let store = SqliteStore::open(&path).unwrap();
    let reviews = store.reviews(task.id()).unwrap();
    let outcomes: Vec<&str> = reviews.iter().map(|r| r.outcome.as_str()).collect();
    assert_eq!(outcomes, ["done", "needs_work"]);
    assert_eq!(reviews[0].model.as_deref(), Some("claude-sonnet-5-5"));
}

#[test]
fn a_deleted_task_takes_its_reviews_with_it() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("looji.db");
    let store = SqliteStore::open(&path).unwrap();
    let task = task_in(&store, "Fix login");
    store.add_review(task.id(), &record("done", 1)).unwrap();
    TaskRepository::delete(&store, task.id()).unwrap();
    assert!(store.reviews(task.id()).unwrap().is_empty());
    let left: i64 = Connection::open(&path)
        .unwrap()
        .query_row("SELECT COUNT(*) FROM task_reviews", [], |row| row.get(0))
        .unwrap();
    assert_eq!(left, 0);
}
