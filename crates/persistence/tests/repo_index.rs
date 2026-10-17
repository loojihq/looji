//! The cached index: what a refresh parses, what it reuses, and what it removes. Also the retrieval of hunks for tasks,
//! checked against the sample app in `tests/fixtures/app`. Each test copies the fixture first, since some of them
//! edit or delete files.

use std::{
    fs,
    path::{Path, PathBuf},
};

use looji_domain::{Project, ProjectId, ProjectRepository, Workspace, WorkspaceRepository};
use looji_persistence::{Connection, SqliteStore};
use tempfile::TempDir;

struct Repo {
    dir: TempDir,
    root: PathBuf,
    project: ProjectId,
}

impl Repo {
    fn db_path(&self) -> PathBuf {
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

/// A database with one project, and a copy of the sample app as that project's repository.
fn repo() -> Repo {
    let dir = TempDir::new().unwrap();
    let root = dir.path().join("app");
    copy_dir(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/app"),
        &root,
    );
    let store = SqliteStore::open(&dir.path().join("looji.db")).unwrap();
    let workspace = Workspace::new("Work").unwrap();
    WorkspaceRepository::save(&store, &workspace).unwrap();
    let project = Project::new(workspace.id(), "app").unwrap();
    ProjectRepository::save(&store, &project).unwrap();
    Repo {
        dir,
        root,
        project: project.id(),
    }
}

#[test]
fn a_first_refresh_parses_every_source_file_and_caches_their_symbols() {
    let repo = repo();
    let store = repo.open();

    let report = store.refresh_repo_index(repo.project, &repo.root).unwrap();

    assert_eq!(
        report.files_seen, 4,
        "login.rs, invoice.py, button.ts and test_invoice.py"
    );
    assert_eq!(report.parsed, 4);
    assert_eq!(report.reused, 0);
    assert!(report.skipped.is_empty(), "{:?}", report.skipped);
    let cached = store.load_repo_index(repo.project).unwrap();
    assert!(cached.symbols.iter().any(|symbol| symbol.name == "login"));
    assert!(
        cached
            .symbols
            .iter()
            .any(|symbol| symbol.name == "renderButton")
    );
    assert_eq!(cached.files.len(), 4);
}

#[test]
fn a_second_refresh_of_an_unchanged_repo_parses_nothing() {
    let repo = repo();
    let store = repo.open();
    assert!(
        store
            .load_repo_index(repo.project)
            .unwrap()
            .symbols
            .is_empty(),
        "nothing is cached before the first refresh"
    );
    let first = store.refresh_repo_index(repo.project, &repo.root).unwrap();
    let cached_after_first = store.load_repo_index(repo.project).unwrap();

    let again = store.refresh_repo_index(repo.project, &repo.root).unwrap();

    assert_eq!(
        again.parsed, 0,
        "an unchanged repository is not parsed again"
    );
    assert_eq!(again.reused, first.files_seen);
    assert_eq!(again.removed, 0);
    assert_eq!(
        store.load_repo_index(repo.project).unwrap(),
        cached_after_first
    );
}

#[test]
fn a_changed_file_is_the_only_one_parsed_and_its_symbols_are_replaced() {
    let repo = repo();
    let store = repo.open();
    store.refresh_repo_index(repo.project, &repo.root).unwrap();

    let login = repo.root.join("src/auth/login.rs");
    let mut source = fs::read_to_string(&login).unwrap();
    source.push_str("\npub fn logout(session: Session) {\n    drop(session);\n}\n");
    fs::write(&login, source).unwrap();

    let report = store.refresh_repo_index(repo.project, &repo.root).unwrap();

    assert_eq!(report.parsed, 1);
    assert_eq!(report.reused, 3);
    let cached = store.load_repo_index(repo.project).unwrap();
    assert!(cached.symbols.iter().any(|symbol| symbol.name == "logout"));
}

#[test]
fn a_deleted_file_loses_its_symbols_and_its_place_in_the_tree() {
    let repo = repo();
    let store = repo.open();
    store.refresh_repo_index(repo.project, &repo.root).unwrap();
    assert!(
        repo.count("SELECT COUNT(*) FROM index_symbols WHERE path = 'src/ui/button.ts'") > 0,
        "the file had symbols to begin with"
    );

    fs::remove_file(repo.root.join("src/ui/button.ts")).unwrap();
    let report = store.refresh_repo_index(repo.project, &repo.root).unwrap();

    assert_eq!(report.removed, 1);
    assert_eq!(
        repo.count("SELECT COUNT(*) FROM index_symbols WHERE path = 'src/ui/button.ts'"),
        0,
        "the symbols go with the file"
    );
    assert_eq!(
        repo.count("SELECT COUNT(*) FROM index_files WHERE path = 'src/ui/button.ts'"),
        0
    );
    let cached = store.load_repo_index(repo.project).unwrap();
    assert!(
        !cached
            .symbols
            .iter()
            .any(|symbol| symbol.name == "renderButton")
    );
    assert!(!cached.files.iter().any(|path| path == "src/ui/button.ts"));
}

#[test]
fn a_file_that_does_not_parse_is_skipped_with_its_reason_and_not_parsed_again() {
    let repo = repo();
    let store = repo.open();
    fs::write(repo.root.join("src/broken.py"), "def broken(:\n    pass\n").unwrap();

    let first = store.refresh_repo_index(repo.project, &repo.root).unwrap();
    assert_eq!(first.skipped.len(), 1);
    assert_eq!(first.skipped[0].path, "src/broken.py");
    assert!(
        first.skipped[0].reason.contains("syntax error"),
        "{}",
        first.skipped[0].reason
    );

    let second = store.refresh_repo_index(repo.project, &repo.root).unwrap();
    assert_eq!(second.parsed, 0, "the failed parse is cached too");
    assert_eq!(second.skipped.len(), 1, "and still reported");
    assert!(
        store
            .load_repo_index(repo.project)
            .unwrap()
            .symbols
            .iter()
            .all(|symbol| symbol.path != "src/broken.py")
    );
}

#[test]
fn a_cached_parse_from_an_older_parser_is_read_again() {
    let repo = repo();
    let store = repo.open();
    store.refresh_repo_index(repo.project, &repo.root).unwrap();
    Connection::open(repo.db_path())
        .unwrap()
        .execute(
            "UPDATE index_files SET parser_version = 0 WHERE path = 'src/auth/login.rs'",
            [],
        )
        .unwrap();

    let report = store.refresh_repo_index(repo.project, &repo.root).unwrap();

    assert_eq!(report.parsed, 1, "only the file with the old parse");
}

#[test]
fn the_cache_survives_a_restart() {
    let repo = repo();
    repo.open()
        .refresh_repo_index(repo.project, &repo.root)
        .unwrap();
    let before = repo.open().load_repo_index(repo.project).unwrap();

    let reopened = repo.open();
    let report = reopened
        .refresh_repo_index(repo.project, &repo.root)
        .unwrap();

    assert_eq!(report.parsed, 0);
    assert_eq!(reopened.load_repo_index(repo.project).unwrap(), before);
}

#[test]
fn deleting_the_project_removes_its_cache() {
    let repo = repo();
    let store = repo.open();
    store.refresh_repo_index(repo.project, &repo.root).unwrap();

    ProjectRepository::delete(&store, repo.project).unwrap();

    assert_eq!(repo.count("SELECT COUNT(*) FROM index_symbols"), 0);
    assert_eq!(repo.count("SELECT COUNT(*) FROM index_files"), 0);
    assert_eq!(repo.count("SELECT COUNT(*) FROM index_entries"), 0);
}

#[test]
fn a_task_returns_the_files_it_is_about_at_the_top() {
    let repo = repo();
    let store = repo.open();
    store.refresh_repo_index(repo.project, &repo.root).unwrap();

    let login = store
        .retrieve_hunks(
            repo.project,
            &repo.root,
            "Fix password verification in login",
            "",
            2_000,
        )
        .unwrap();
    assert_eq!(
        login.first().map(|hunk| hunk.path.as_str()),
        Some("src/auth/login.rs")
    );
    assert!(login[0].text.contains("fn login"), "{}", login[0].text);

    let invoice = store
        .retrieve_hunks(
            repo.project,
            &repo.root,
            "Include the due date in the invoice total",
            "Totals should show when payment is due.",
            2_000,
        )
        .unwrap();
    assert_eq!(
        invoice.first().map(|hunk| hunk.path.as_str()),
        Some("src/billing/invoice.py")
    );
    assert!(
        invoice[0].text.contains("def total_due"),
        "{}",
        invoice[0].text
    );
}

#[test]
fn a_task_with_no_matching_code_gets_no_hunks() {
    let repo = repo();
    let store = repo.open();
    store.refresh_repo_index(repo.project, &repo.root).unwrap();

    let hunks = store
        .retrieve_hunks(
            repo.project,
            &repo.root,
            "Recolour the weather widget gradient",
            "Use a softer colour on the forecast panel.",
            2_000,
        )
        .unwrap();

    assert!(hunks.is_empty(), "{hunks:?}");
}

#[test]
fn hunks_stay_within_the_token_budget() {
    let repo = repo();
    let store = repo.open();
    store.refresh_repo_index(repo.project, &repo.root).unwrap();

    let hunks = store
        .retrieve_hunks(repo.project, &repo.root, "invoice total due", "", 60)
        .unwrap();

    let used: usize = hunks
        .iter()
        .map(|hunk| looji_domain::estimate_tokens(&hunk.text))
        .sum();
    assert!(!hunks.is_empty(), "a hunk fits in the budget");
    assert!(used <= 60, "{used} tokens over a budget of 60");
}
