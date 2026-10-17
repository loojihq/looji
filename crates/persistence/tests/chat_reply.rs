//! How a reply ends is kept: a stopped reply keeps its partial text, a failed one keeps its text and reason, and a
//! retry removes the failed reply so a new one can take its place.

use looji_domain::{
    MessageRole, Position, Project, ProjectRepository, ReplyState, Status, Task,
    TaskChatRepository, TaskId, TaskRepository, Workspace, WorkspaceRepository,
};
use looji_persistence::SqliteStore;
use tempfile::TempDir;

fn store_with_task() -> (TempDir, SqliteStore, TaskId) {
    let dir = TempDir::new().unwrap();
    let store = SqliteStore::open(&dir.path().join("looji.db")).unwrap();
    let workspace = Workspace::new("Work").unwrap();
    WorkspaceRepository::save(&store, &workspace).unwrap();
    let project = Project::new(workspace.id(), "looji").unwrap();
    ProjectRepository::save(&store, &project).unwrap();
    let task = Task::new(
        project.id(),
        "Fix login",
        Status::Todo,
        Position::new("a0").unwrap(),
    )
    .unwrap();
    TaskRepository::save(&store, &task).unwrap();
    (dir, store, task.id())
}

#[test]
fn a_stopped_reply_keeps_its_partial_text_across_a_restart() {
    let (dir, store, task) = store_with_task();
    let thread = store.start_thread(task).unwrap();
    store
        .append_message(thread, MessageRole::User, "Explain", None)
        .unwrap();
    let reply = store
        .append_message(
            thread,
            MessageRole::Assistant,
            "",
            Some("claude-sonnet-5-5"),
        )
        .unwrap();
    store
        .finish_message(thread, reply, "The login checks", &ReplyState::Stopped)
        .unwrap();
    drop(store);

    let reopened = SqliteStore::open(&dir.path().join("looji.db")).unwrap();
    let messages = reopened.messages(thread).unwrap();

    assert_eq!(messages[1].content, "The login checks");
    assert_eq!(messages[1].state, ReplyState::Stopped);
    assert_eq!(messages[1].model.as_deref(), Some("claude-sonnet-5-5"));
}

#[test]
fn a_failed_reply_keeps_its_text_and_reason() {
    let (_dir, store, task) = store_with_task();
    let thread = store.start_thread(task).unwrap();
    let reply = store
        .append_message(thread, MessageRole::Assistant, "", None)
        .unwrap();
    let state = ReplyState::Failed {
        reason: "network error: connection reset".to_string(),
    };

    store
        .finish_message(thread, reply, "Partly", &state)
        .unwrap();

    let messages = store.messages(thread).unwrap();
    assert_eq!(messages[0].content, "Partly");
    assert_eq!(messages[0].state, state);
}

#[test]
fn retrying_removes_the_failed_reply_and_anything_after_it() {
    let (_dir, store, task) = store_with_task();
    let thread = store.start_thread(task).unwrap();
    store
        .append_message(thread, MessageRole::User, "Why?", None)
        .unwrap();
    let failed = store
        .append_message(thread, MessageRole::Assistant, "", None)
        .unwrap();
    store
        .finish_message(
            thread,
            failed,
            "Because",
            &ReplyState::Failed {
                reason: "dropped".to_string(),
            },
        )
        .unwrap();

    store.truncate_from(thread, failed).unwrap();

    let messages = store.messages(thread).unwrap();
    assert_eq!(messages.len(), 1, "only the question is left");
    assert_eq!(messages[0].role, MessageRole::User);
    let retry = store
        .append_message(thread, MessageRole::Assistant, "", None)
        .unwrap();
    assert_eq!(retry, failed, "the new reply takes the failed one's place");
}

#[test]
fn the_user_message_is_always_complete_and_positions_count_up() {
    let (_dir, store, task) = store_with_task();
    let thread = store.start_thread(task).unwrap();
    let first = store
        .append_message(thread, MessageRole::User, "One", None)
        .unwrap();
    let second = store
        .append_message(thread, MessageRole::Assistant, "Two", None)
        .unwrap();

    assert_eq!((first, second), (0, 1));
    let messages = store.messages(thread).unwrap();
    assert!(
        messages
            .iter()
            .all(|message| message.state == ReplyState::Complete)
    );
}
