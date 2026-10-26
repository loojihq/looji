use std::{fs, path::Path};

use looji_domain::{
    Attachment, AttachmentRepository, MediaKind, Position, Project, ProjectRepository, Status,
    Task, TaskId, TaskRepository, Timestamp, Workspace, WorkspaceRepository,
};
use looji_persistence::{
    SqliteStore,
    attachments::{AttachmentError, MAX_ATTACHMENT_BYTES, attachments_dir, import_attachment},
};
use tempfile::TempDir;

/// A store with one project and one task on its board.
fn store_with_task(path: &Path) -> (SqliteStore, TaskId) {
    let store = SqliteStore::open(path).unwrap();
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
    (store, id)
}

fn attachment(task: TaskId, stored_name: &str, file_name: &str) -> Attachment {
    Attachment {
        task,
        stored_name: stored_name.to_string(),
        file_name: file_name.to_string(),
        kind: MediaKind::from_file_name(file_name),
        created_at: Timestamp::from_unix_millis(1_000),
    }
}

#[test]
fn attachments_are_listed_oldest_first_and_removed_by_name() {
    let temp = TempDir::new().unwrap();
    let (store, task) = store_with_task(&temp.path().join("looji.db"));

    let later = Attachment {
        created_at: Timestamp::from_unix_millis(2_000),
        ..attachment(task, "b.mp4", "demo.mp4")
    };
    store.add(&later).unwrap();
    store.add(&attachment(task, "a.png", "screen.png")).unwrap();

    let listed = store.list_for_task(task).unwrap();
    let names: Vec<&str> = listed
        .iter()
        .map(|item| item.stored_name.as_str())
        .collect();
    assert_eq!(names, ["a.png", "b.mp4"]);
    assert_eq!(listed[0].kind, MediaKind::Image);
    assert_eq!(listed[1].kind, MediaKind::Video);

    store.remove(task, "a.png").unwrap();
    let remaining = store.list_for_task(task).unwrap();
    assert_eq!(remaining, vec![later]);
}

#[test]
fn deleting_a_task_takes_its_attachment_rows_with_it() {
    let temp = TempDir::new().unwrap();
    let (store, task) = store_with_task(&temp.path().join("looji.db"));
    store.add(&attachment(task, "a.pdf", "spec.pdf")).unwrap();

    TaskRepository::delete(&store, task).unwrap();

    assert!(store.list_for_task(task).unwrap().is_empty());
}

#[test]
fn a_picked_file_is_copied_into_the_attachments_folder_and_recorded() {
    let temp = TempDir::new().unwrap();
    let (store, task) = store_with_task(&temp.path().join("looji.db"));
    let source = temp.path().join("Walkthrough.MP4");
    fs::write(&source, b"frames").unwrap();
    let folder = attachments_dir(temp.path());

    let copied = import_attachment(&source, &folder).unwrap();
    store
        .add(&Attachment {
            task,
            stored_name: copied.stored_name.clone(),
            file_name: copied.file_name.clone(),
            kind: MediaKind::from_file_name(&copied.file_name),
            created_at: Timestamp::from_unix_millis(3_000),
        })
        .unwrap();

    assert_eq!(copied.file_name, "Walkthrough.MP4");
    assert_eq!(
        fs::read(folder.join(&copied.stored_name)).unwrap(),
        b"frames"
    );
    let listed = store.list_for_task(task).unwrap();
    assert_eq!(listed[0].kind, MediaKind::Video);
    assert_eq!(listed[0].stored_name, copied.stored_name);
}

#[test]
fn a_file_over_the_size_limit_is_refused_and_nothing_is_written() {
    let temp = TempDir::new().unwrap();
    let source = temp.path().join("huge.mov");
    let file = fs::File::create(&source).unwrap();
    file.set_len(MAX_ATTACHMENT_BYTES + 1).unwrap();
    let folder = attachments_dir(temp.path());

    assert_eq!(
        import_attachment(&source, &folder),
        Err(AttachmentError::TooLarge)
    );
    assert!(!folder.exists());
}
