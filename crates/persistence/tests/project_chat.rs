use looji_domain::{
    MessageRole, Position, Project, ProjectRepository, Status, Task, TaskChatRepository, TaskId,
    TaskRepository, Workspace, WorkspaceRepository,
};
use looji_persistence::SqliteStore;
use tempfile::TempDir;

/// A store with one project and one task on its board.
fn store_with_task(dir: &TempDir) -> (SqliteStore, Project, TaskId) {
    let store = SqliteStore::open(&dir.path().join("looji.db")).unwrap();
    let workspace = Workspace::new("Personal").unwrap();
    WorkspaceRepository::save(&store, &workspace).unwrap();
    let project = Project::new(workspace.id(), "Website").unwrap();
    ProjectRepository::save(&store, &project).unwrap();
    let task = Task::new(
        project.id(),
        "Pricing page",
        Status::Todo,
        Position::new("a0").unwrap(),
    )
    .unwrap();
    let id = task.id();
    TaskRepository::save(&store, &task).unwrap();
    (store, project, id)
}

/// The id a conversation about a whole project is stored under: the project's own.
fn owner(project: &Project) -> TaskId {
    TaskId::from_uuid(project.id().as_uuid())
}

#[test]
fn a_conversation_about_a_project_is_saved_and_comes_back() {
    let dir = TempDir::new().unwrap();
    let (store, project, _) = store_with_task(&dir);

    let thread = store.start_thread(owner(&project)).unwrap();
    store
        .append_message(thread, MessageRole::User, "Where is login handled?", None)
        .unwrap();
    store
        .append_message(
            thread,
            MessageRole::Assistant,
            "In src/auth.",
            Some("model"),
        )
        .unwrap();

    assert_eq!(store.threads(owner(&project)).unwrap(), vec![thread]);
    let messages = store.messages(thread).unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].content, "Where is login handled?");
}

#[test]
fn deleting_a_project_deletes_its_conversation_and_leaves_other_threads_alone() {
    let dir = TempDir::new().unwrap();
    let (store, project, task) = store_with_task(&dir);
    let project_thread = store.start_thread(owner(&project)).unwrap();
    store
        .append_message(project_thread, MessageRole::User, "hello", None)
        .unwrap();
    let other = Project::new(
        looji_domain::WorkspaceRepository::list(&store).unwrap()[0].id(),
        "Mobile",
    )
    .unwrap();
    ProjectRepository::save(&store, &other).unwrap();
    let other_thread = store.start_thread(owner(&other)).unwrap();
    let task_thread = store.start_thread(task).unwrap();

    ProjectRepository::delete(&store, project.id()).unwrap();

    assert!(store.threads(owner(&project)).unwrap().is_empty());
    assert!(store.messages(project_thread).unwrap().is_empty());
    assert_eq!(store.threads(owner(&other)).unwrap(), vec![other_thread]);
    // The task went with its project, and so did its thread.
    assert!(store.threads(task).unwrap().is_empty());
    let _ = task_thread;
}

#[test]
fn deleting_a_task_still_deletes_its_conversation() {
    let dir = TempDir::new().unwrap();
    let (store, project, task) = store_with_task(&dir);
    let task_thread = store.start_thread(task).unwrap();
    store
        .append_message(task_thread, MessageRole::User, "explain this", None)
        .unwrap();
    let project_thread = store.start_thread(owner(&project)).unwrap();

    TaskRepository::delete(&store, task).unwrap();

    assert!(store.threads(task).unwrap().is_empty());
    assert!(store.messages(task_thread).unwrap().is_empty());
    assert_eq!(
        store.threads(owner(&project)).unwrap(),
        vec![project_thread]
    );
}
