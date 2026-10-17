//! The task check and the review notes. The model is a scripted fake, so these tests check what is asked, how the
//! reply is read, and what is kept. They don't check whether a real model judges the code correctly. That needs a
//! key and a live run.

use std::{collections::BTreeSet, fs, path::Path};

use looji_ai::{
    AiError, CancellationToken,
    fake::{FakeProvider, Script},
    review::{
        check::{CHECK_QUESTION, TaskCheck, Verdict, check_task, parse_verdict},
        notes::{NO_TESTS_NOTE, NotesError, add_test_gap_note, parse_notes, review_task},
    },
};
use looji_domain::{
    Hunk, NoteKind, Project, ProjectId, ProjectRepository, ReviewNote, Workspace,
    WorkspaceRepository,
};
use looji_persistence::SqliteStore;
use tempfile::TempDir;

fn fixture_app() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../persistence/tests/fixtures/app")
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

fn hunk(path: &str, start: u32, end: u32, text: &str) -> Hunk {
    Hunk {
        path: path.to_string(),
        start_line: start,
        end_line: end,
        score: 4,
        text: text.to_string(),
    }
}

fn login_hunk() -> Hunk {
    hunk(
        "src/auth/login.rs",
        5,
        13,
        "pub fn login(user: &str, password: &str) -> Option<Session> {\n    if verify_password(user, password) {",
    )
}

fn shown(hunks: &[Hunk]) -> BTreeSet<&str> {
    hunks.iter().map(|hunk| hunk.path.as_str()).collect()
}

// Reading verdicts.

#[test]
fn a_clean_implemented_reply_names_the_file_that_shows_it() {
    let hunks = [login_hunk()];
    let reply = r#"{"verdict": "implemented", "evidence": ["src/auth/login.rs"]}"#;

    let check = parse_verdict(reply, &shown(&hunks));

    assert_eq!(
        check,
        TaskCheck {
            verdict: Verdict::Implemented,
            evidence: vec!["src/auth/login.rs".to_string()],
        }
    );
}

#[test]
fn a_fenced_reply_with_prose_around_it_is_still_read() {
    let hunks = [login_hunk()];
    let reply = "Here is my answer:\n```json\n{\"verdict\": \"partial\", \"evidence\": [\"src/auth/login.rs\"]}\n```";

    assert_eq!(
        parse_verdict(reply, &shown(&hunks)).verdict,
        Verdict::Partial
    );
}

#[test]
fn a_file_the_model_was_not_shown_is_dropped_from_the_evidence() {
    let hunks = [login_hunk()];
    let reply = r#"{"verdict": "partial", "evidence": ["src/billing/invoice.py", "src/auth/login.rs", "src/auth/login.rs"]}"#;

    let check = parse_verdict(reply, &shown(&hunks));

    assert_eq!(check.evidence, ["src/auth/login.rs"]);
}

#[test]
fn not_started_is_read_with_or_without_an_underscore() {
    let hunks = [login_hunk()];
    assert_eq!(
        parse_verdict(
            r#"{"verdict": "not started", "evidence": []}"#,
            &shown(&hunks)
        )
        .verdict,
        Verdict::NotStarted
    );
    assert_eq!(
        parse_verdict(r#"{"verdict": "Not_Started"}"#, &shown(&hunks)).verdict,
        Verdict::NotStarted
    );
}

#[test]
fn an_unparseable_reply_is_unknown_and_never_implemented() {
    let hunks = [login_hunk()];
    for reply in [
        "Yes, it looks implemented to me.",
        "{\"verdict\": \"implemented\"",
        "{\"verdict\": \"done\", \"evidence\": [\"src/auth/login.rs\"]}",
        "{\"evidence\": [\"src/auth/login.rs\"]}",
        "[\"implemented\"]",
        "",
    ] {
        let check = parse_verdict(reply, &shown(&hunks));
        assert_eq!(check.verdict, Verdict::Unknown, "reply {reply:?}");
        assert!(check.evidence.is_empty(), "reply {reply:?}");
    }
}

#[test]
fn implemented_needs_a_file_that_was_shown() {
    let hunks = [login_hunk()];
    let no_evidence = parse_verdict(
        r#"{"verdict": "implemented", "evidence": []}"#,
        &shown(&hunks),
    );
    let only_unshown = parse_verdict(
        r#"{"verdict": "implemented", "evidence": ["src/billing/invoice.py"]}"#,
        &shown(&hunks),
    );

    assert_eq!(no_evidence.verdict, Verdict::Unknown);
    assert_eq!(only_unshown.verdict, Verdict::Unknown);
}

// The check against fixture tasks, through the fake provider.

#[tokio::test]
async fn a_task_the_fixture_implements_is_checked_with_its_code_and_the_fixed_question() {
    let provider = FakeProvider::new();
    provider.push(Script::tokens([
        r#"{"verdict": "implemented", "evidence": ["src/auth/login.rs"]}"#,
    ]));
    let hunks = [login_hunk()];

    let check = check_task(
        &provider,
        "claude-sonnet-5-5",
        "Fix password verification in login",
        "",
        &hunks,
        CancellationToken::new(),
    )
    .await
    .unwrap();

    assert_eq!(check.verdict, Verdict::Implemented);
    assert_eq!(check.evidence, ["src/auth/login.rs"]);
    let sent = provider.requests();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].system.as_deref(), Some(CHECK_QUESTION));
    let body = &sent[0].messages[0].content;
    assert!(
        body.contains("Fix password verification in login"),
        "{body}"
    );
    assert!(body.contains("### src/auth/login.rs:5-13"), "{body}");
    assert!(body.contains("fn login"), "{body}");
}

#[tokio::test]
async fn a_task_the_fixture_does_not_touch_is_reported_as_not_started() {
    let provider = FakeProvider::new();
    provider.push(Script::tokens([
        r#"{"verdict": "not_started", "evidence": []}"#,
    ]));

    let check = check_task(
        &provider,
        "claude-sonnet-5-5",
        "Add PDF export for invoices",
        "",
        &[],
        CancellationToken::new(),
    )
    .await
    .unwrap();

    assert_eq!(check.verdict, Verdict::NotStarted);
    assert!(
        provider.requests()[0].messages[0]
            .content
            .contains("(no code matched this task)")
    );
}

#[tokio::test]
async fn a_prose_reply_to_the_check_is_unknown() {
    let provider = FakeProvider::new();
    provider.push(Script::tokens(["It is mostly done, I think."]));

    let check = check_task(
        &provider,
        "claude-sonnet-5-5",
        "Fix password verification in login",
        "",
        &[login_hunk()],
        CancellationToken::new(),
    )
    .await
    .unwrap();

    assert_eq!(check.verdict, Verdict::Unknown);
}

#[tokio::test]
async fn a_failed_request_is_an_error_not_a_verdict() {
    let provider = FakeProvider::new();
    provider.push(Script::refused(AiError::AuthFailed));

    let result = check_task(
        &provider,
        "claude-sonnet-5-5",
        "Fix password verification in login",
        "",
        &[login_hunk()],
        CancellationToken::new(),
    )
    .await;

    assert!(matches!(result, Err(AiError::AuthFailed)));
}

#[tokio::test]
async fn the_hunks_come_from_retrieval_over_the_fixture() {
    // The check's input is what retrieval returns for the task, read from a real indexed copy of the fixture.
    let dir = TempDir::new().unwrap();
    let root = dir.path().join("app");
    copy_dir(&fixture_app(), &root);
    let store = SqliteStore::open(&dir.path().join("looji.db")).unwrap();
    let workspace = Workspace::new("Work").unwrap();
    WorkspaceRepository::save(&store, &workspace).unwrap();
    let project = Project::new(workspace.id(), "app").unwrap();
    ProjectRepository::save(&store, &project).unwrap();
    let project_id: ProjectId = project.id();
    store.refresh_repo_index(project_id, &root).unwrap();
    let title = "Fix password verification in login";
    let hunks = store
        .retrieve_hunks(project_id, &root, title, "", 2_000)
        .unwrap();
    assert_eq!(
        hunks.first().map(|hunk| hunk.path.as_str()),
        Some("src/auth/login.rs")
    );

    let provider = FakeProvider::new();
    provider.push(Script::tokens([
        r#"{"verdict": "implemented", "evidence": ["src/auth/login.rs"]}"#,
    ]));
    let check = check_task(
        &provider,
        "claude-sonnet-5-5",
        title,
        "",
        &hunks,
        CancellationToken::new(),
    )
    .await
    .unwrap();

    assert_eq!(check.verdict, Verdict::Implemented);
}

// Review notes.

const APP_FILES: &[&str] = &[
    "src/auth/login.rs",
    "src/billing/invoice.py",
    "src/ui/button.ts",
];

fn index_files(with_tests: bool) -> Vec<String> {
    let mut files: Vec<String> = APP_FILES.iter().map(|path| path.to_string()).collect();
    if with_tests {
        files.push("tests/test_invoice.py".to_string());
    }
    files
}

#[test]
fn notes_keep_only_the_files_in_the_index() {
    let known: BTreeSet<&str> = APP_FILES.iter().copied().collect();
    let reply = r#"{"notes": [
        {"kind": "risk", "text": "Password check is loose.", "files": ["src/auth/login.rs", "src/ghost.rs"]},
        {"kind": "check", "text": "Confirm the session expires.", "files": ["src/invented/session.rs"]}
    ]}"#;

    let notes = parse_notes(reply, &known).unwrap();

    assert_eq!(
        notes.len(),
        2,
        "a note whose file is unknown is kept, without the file"
    );
    assert_eq!(notes[0].files, ["src/auth/login.rs"]);
    assert!(notes[1].files.is_empty());
    assert!(
        notes
            .iter()
            .all(|note| note.files.iter().all(|file| known.contains(file.as_str())))
    );
}

#[test]
fn notes_with_an_unknown_kind_or_no_text_are_left_out() {
    let known: BTreeSet<&str> = APP_FILES.iter().copied().collect();
    let reply = r#"{"notes": [
        {"kind": "praise", "text": "Nice."},
        {"kind": "risk", "text": "   "},
        {"kind": "missing_test", "text": "No test covers login failures.", "files": []}
    ]}"#;

    let notes = parse_notes(reply, &known).unwrap();

    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].kind, NoteKind::MissingTest);
}

#[test]
fn a_reply_without_a_notes_list_is_unreadable() {
    let known: BTreeSet<&str> = APP_FILES.iter().copied().collect();
    assert_eq!(
        parse_notes("Looks fine.", &known),
        Err(NotesError::Unreadable)
    );
    assert_eq!(
        parse_notes(r#"{"notes": "none"}"#, &known),
        Err(NotesError::Unreadable)
    );
}

#[test]
fn a_repository_with_no_tests_gets_a_note_saying_so() {
    let mut notes: Vec<ReviewNote> = Vec::new();

    add_test_gap_note(&mut notes, &index_files(false));

    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].kind, NoteKind::MissingTest);
    assert_eq!(notes[0].body, NO_TESTS_NOTE);
}

#[test]
fn a_repository_with_tests_gets_no_such_note() {
    let mut notes: Vec<ReviewNote> = Vec::new();

    add_test_gap_note(&mut notes, &index_files(true));

    assert!(notes.is_empty());
}

#[tokio::test]
async fn review_notes_for_a_repo_with_no_tests_say_so_and_name_only_indexed_files() {
    let provider = FakeProvider::new();
    provider.push(Script::tokens([r#"{"notes": [
        {"kind": "risk", "text": "Verification ignores empty passwords.", "files": ["src/auth/login.rs", "src/gone.rs"]}
    ]}"#]));

    let notes = review_task(
        &provider,
        "claude-sonnet-5-5",
        "Fix password verification in login",
        "",
        &[login_hunk()],
        &index_files(false),
        CancellationToken::new(),
    )
    .await
    .unwrap();

    assert!(
        notes
            .iter()
            .any(|note| note.kind == NoteKind::MissingTest && note.body == NO_TESTS_NOTE)
    );
    let indexed: BTreeSet<String> = index_files(false).into_iter().collect();
    for note in &notes {
        for file in &note.files {
            assert!(
                indexed.contains(file),
                "note names {file}, which isn't in the index"
            );
        }
    }
}

#[tokio::test]
async fn an_unreadable_notes_reply_is_an_error() {
    let provider = FakeProvider::new();
    provider.push(Script::tokens(["No notes today."]));

    let result = review_task(
        &provider,
        "claude-sonnet-5-5",
        "Fix password verification in login",
        "",
        &[login_hunk()],
        &index_files(true),
        CancellationToken::new(),
    )
    .await;

    assert!(matches!(result, Err(AiError::Protocol(_))));
}
