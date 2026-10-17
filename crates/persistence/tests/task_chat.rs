//! Conversations about a task and its review notes. Each restart is a real close and reopen of the database file, so
//! what's checked is what's on disk.

use looji_domain::{
    ChatThreadId, MessageRole, NoteKind, Position, Project, ProjectRepository, ReviewNote,
    ReviewNoteRepository, Status, Task, TaskChatRepository, TaskId, TaskRepository, Workspace,
    WorkspaceRepository,
};
use looji_persistence::{Connection, SqliteStore};
use tempfile::TempDir;

struct Fixture {
    dir: TempDir,
    task: TaskId,
}

impl Fixture {
    fn db_path(&self) -> std::path::PathBuf {
        self.dir.path().join("looji.db")
    }

    fn open(&self) -> SqliteStore {
        SqliteStore::open(&self.db_path()).unwrap()
    }

    fn count(&self, sql: &str) -> i64 {
        Connection::open(self.db_path())
            .unwrap()
            .query_row(sql, [], |row| row.get(0))
            .unwrap()
    }
}

/// A database with one task in it, created and closed.
fn fixture() -> Fixture {
    let dir = TempDir::new().unwrap();
    let store = SqliteStore::open(&dir.path().join("looji.db")).unwrap();
    let workspace = Workspace::new("Work").unwrap();
    WorkspaceRepository::save(&store, &workspace).unwrap();
    let project = Project::new(workspace.id(), "looji").unwrap();
    ProjectRepository::save(&store, &project).unwrap();
    let task = Task::new(
        project.id(),
        "Fix password verification",
        Status::Todo,
        Position::new("a0").unwrap(),
    )
    .unwrap();
    TaskRepository::save(&store, &task).unwrap();
    Fixture {
        dir,
        task: task.id(),
    }
}

#[test]
fn a_conversation_reloads_in_the_order_it_was_written_after_a_restart() {
    let fixture = fixture();
    let thread = {
        let store = fixture.open();
        let thread = store.start_thread(fixture.task).unwrap();
        store
            .append_message(
                thread,
                MessageRole::User,
                "Does login check the password?",
                None,
            )
            .unwrap();
        store
            .append_message(
                thread,
                MessageRole::Assistant,
                "Yes, verify_password runs first.",
                Some("claude-sonnet-5-5"),
            )
            .unwrap();
        store
            .append_message(thread, MessageRole::User, "And an empty one?", None)
            .unwrap();
        thread
    };

    let reopened = fixture.open();
    let messages = reopened.messages(thread).unwrap();

    let roles: Vec<MessageRole> = messages.iter().map(|message| message.role).collect();
    assert_eq!(
        roles,
        [MessageRole::User, MessageRole::Assistant, MessageRole::User]
    );
    let contents: Vec<&str> = messages
        .iter()
        .map(|message| message.content.as_str())
        .collect();
    assert_eq!(
        contents,
        [
            "Does login check the password?",
            "Yes, verify_password runs first.",
            "And an empty one?",
        ]
    );
    assert_eq!(
        messages[0].model, None,
        "the user's own messages have no model"
    );
    assert_eq!(messages[1].model.as_deref(), Some("claude-sonnet-5-5"));
    assert!(messages[0].created_at.unix_millis() > 0);
}

#[test]
fn threads_are_listed_per_task_oldest_first() {
    let fixture = fixture();
    let store = fixture.open();
    let first = store.start_thread(fixture.task).unwrap();
    let second = store.start_thread(fixture.task).unwrap();

    let reopened = fixture.open();
    assert_eq!(reopened.threads(fixture.task).unwrap(), [first, second]);
    assert!(reopened.threads(TaskId::new()).unwrap().is_empty());
}

#[test]
fn deleting_the_task_deletes_its_conversation() {
    let fixture = fixture();
    let store = fixture.open();
    let thread = store.start_thread(fixture.task).unwrap();
    store
        .append_message(thread, MessageRole::User, "A question", None)
        .unwrap();

    TaskRepository::delete(&store, fixture.task).unwrap();

    assert_eq!(fixture.count("SELECT COUNT(*) FROM task_threads"), 0);
    assert_eq!(fixture.count("SELECT COUNT(*) FROM task_messages"), 0);
    assert!(store.messages(thread).unwrap().is_empty());
}

#[test]
fn a_message_for_a_thread_that_does_not_exist_is_refused() {
    let fixture = fixture();
    let result =
        fixture
            .open()
            .append_message(ChatThreadId::new(), MessageRole::User, "orphan", None);
    assert!(result.is_err());
}

#[test]
fn review_notes_reload_in_order_with_their_files_after_a_restart() {
    let fixture = fixture();
    let notes = vec![
        ReviewNote {
            kind: NoteKind::Risk,
            body: "Empty passwords may pass.".to_string(),
            files: vec!["src/auth/login.rs".to_string()],
        },
        ReviewNote {
            kind: NoteKind::MissingTest,
            body: "No test covers failed logins.".to_string(),
            files: Vec::new(),
        },
    ];
    fixture
        .open()
        .replace_notes(fixture.task, &notes, Some("claude-sonnet-5-5"))
        .unwrap();

    assert_eq!(fixture.open().notes(fixture.task).unwrap(), notes);
}

#[test]
fn making_notes_again_replaces_the_earlier_ones() {
    let fixture = fixture();
    let store = fixture.open();
    let first = vec![ReviewNote {
        kind: NoteKind::Check,
        body: "Old note".to_string(),
        files: Vec::new(),
    }];
    store.replace_notes(fixture.task, &first, None).unwrap();
    let second = vec![ReviewNote {
        kind: NoteKind::Risk,
        body: "New note".to_string(),
        files: Vec::new(),
    }];

    store.replace_notes(fixture.task, &second, None).unwrap();

    assert_eq!(fixture.open().notes(fixture.task).unwrap(), second);
}

#[test]
fn deleting_the_task_deletes_its_review_notes() {
    let fixture = fixture();
    let store = fixture.open();
    store
        .replace_notes(
            fixture.task,
            &[ReviewNote {
                kind: NoteKind::Check,
                body: "Check it".to_string(),
                files: Vec::new(),
            }],
            None,
        )
        .unwrap();

    TaskRepository::delete(&store, fixture.task).unwrap();

    assert_eq!(fixture.count("SELECT COUNT(*) FROM task_review_notes"), 0);
}

#[test]
fn a_role_or_kind_stored_outside_the_known_values_is_refused_by_storage() {
    let fixture = fixture();
    let thread = fixture.open().start_thread(fixture.task).unwrap();
    let inserted = Connection::open(fixture.db_path()).unwrap().execute(
        "INSERT INTO task_messages (id, thread_id, position, role, content, created_at)
         VALUES ('m', ?1, 0, 'system', 'x', 0)",
        [thread.as_uuid().to_string()],
    );
    assert!(inserted.is_err());
}
