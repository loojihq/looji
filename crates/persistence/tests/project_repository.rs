//! A project's repository folder: what the disk check reports, and what is stored across restarts. Each restart is a
//! real close and reopen of the database file.

use std::{fs, path::Path};

use looji_domain::{
    CodeSourceRepository, PathCheck, Project, ProjectId, ProjectRepository, Workspace,
    WorkspaceRepository,
};
use looji_persistence::{SqliteStore, repo_folder::check_repo_folder};
use tempfile::TempDir;

struct Fixture {
    dir: TempDir,
    project: ProjectId,
    workspace: looji_domain::WorkspaceId,
}

impl Fixture {
    fn db_path(&self) -> std::path::PathBuf {
        self.dir.path().join("looji.db")
    }

    fn open(&self) -> SqliteStore {
        SqliteStore::open(&self.db_path()).unwrap()
    }
}

/// A database with one project in it, created and closed, so tests reopen the same file.
fn fixture() -> Fixture {
    let dir = TempDir::new().unwrap();
    let store = SqliteStore::open(&dir.path().join("looji.db")).unwrap();
    let workspace = Workspace::new("Work").unwrap();
    WorkspaceRepository::save(&store, &workspace).unwrap();
    let project = Project::new(workspace.id(), "looji").unwrap();
    ProjectRepository::save(&store, &project).unwrap();
    Fixture {
        dir,
        project: project.id(),
        workspace: workspace.id(),
    }
}

/// A folder that looks like the top of a git repository.
fn repo_folder(parent: &Path, name: &str) -> std::path::PathBuf {
    let folder = parent.join(name);
    fs::create_dir_all(folder.join(".git")).unwrap();
    folder
}

#[test]
fn a_folder_with_a_git_directory_is_accepted() {
    let dir = TempDir::new().unwrap();
    let folder = repo_folder(dir.path(), "repo");
    assert_eq!(check_repo_folder(&folder), Ok(()));
}

#[test]
fn a_folder_with_a_git_file_is_accepted_for_worktrees_and_submodules() {
    let dir = TempDir::new().unwrap();
    let folder = dir.path().join("worktree");
    fs::create_dir(&folder).unwrap();
    fs::write(
        folder.join(".git"),
        "gitdir: ../repo/.git/worktrees/worktree\n",
    )
    .unwrap();
    assert_eq!(check_repo_folder(&folder), Ok(()));
}

#[test]
fn a_folder_that_does_not_exist_is_missing() {
    let dir = TempDir::new().unwrap();
    assert_eq!(
        check_repo_folder(&dir.path().join("nope")),
        Err(PathCheck::Missing)
    );
}

#[test]
fn a_file_is_not_a_folder() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("notes.txt");
    fs::write(&file, "hello").unwrap();
    assert_eq!(check_repo_folder(&file), Err(PathCheck::NotADirectory));
}

#[test]
fn a_folder_without_a_git_entry_is_not_a_repository() {
    let dir = TempDir::new().unwrap();
    let folder = dir.path().join("plain");
    fs::create_dir(&folder).unwrap();
    assert_eq!(
        check_repo_folder(&folder),
        Err(PathCheck::NotAGitRepository)
    );
}

#[test]
fn a_subfolder_of_a_repository_is_not_the_repository_root() {
    let dir = TempDir::new().unwrap();
    let root = repo_folder(dir.path(), "repo");
    let inner = root.join("src");
    fs::create_dir(&inner).unwrap();
    assert_eq!(check_repo_folder(&inner), Err(PathCheck::NotAGitRepository));
}

#[test]
fn a_saved_path_is_read_back_after_a_restart() {
    let fixture = fixture();
    let folder = repo_folder(fixture.dir.path(), "repo");
    let path = folder.to_string_lossy().into_owned();

    fixture.open().save_path(fixture.project, &path).unwrap();

    let reopened = fixture.open();
    assert_eq!(reopened.load_path(fixture.project).unwrap(), Some(path));
}

#[test]
fn a_project_with_no_saved_path_is_not_connected() {
    let fixture = fixture();
    assert_eq!(fixture.open().load_path(fixture.project).unwrap(), None);
}

#[test]
fn saving_again_replaces_the_path() {
    let fixture = fixture();
    let store = fixture.open();
    store.save_path(fixture.project, "C:\\old").unwrap();
    store.save_path(fixture.project, "C:\\new").unwrap();
    assert_eq!(
        store.load_path(fixture.project).unwrap(),
        Some("C:\\new".to_string())
    );
}

#[test]
fn a_folder_that_disappears_keeps_its_saved_path_and_reports_missing() {
    let fixture = fixture();
    let folder = repo_folder(fixture.dir.path(), "repo");
    let path = folder.to_string_lossy().into_owned();
    fixture.open().save_path(fixture.project, &path).unwrap();

    fs::remove_dir_all(&folder).unwrap();

    let reopened = fixture.open();
    assert_eq!(
        reopened.load_path(fixture.project).unwrap(),
        Some(path.clone()),
        "losing the folder must not clear the saved path"
    );
    assert_eq!(check_repo_folder(Path::new(&path)), Err(PathCheck::Missing));
}

#[test]
fn only_clear_removes_the_saved_path() {
    let fixture = fixture();
    let store = fixture.open();
    store.save_path(fixture.project, "C:\\repo").unwrap();

    store.clear_path(fixture.project).unwrap();

    assert_eq!(store.load_path(fixture.project).unwrap(), None);
}

#[test]
fn an_empty_path_is_refused_by_storage() {
    let fixture = fixture();
    assert!(fixture.open().save_path(fixture.project, "   ").is_err());
}

#[test]
fn deleting_the_project_removes_its_path() {
    let fixture = fixture();
    let store = fixture.open();
    store.save_path(fixture.project, "C:\\repo").unwrap();

    ProjectRepository::delete(&store, fixture.project).unwrap();

    assert_eq!(store.load_path(fixture.project).unwrap(), None);
    drop(store);
    let rows: i64 = rusqlite_count(&fixture, "SELECT COUNT(*) FROM project_repository");
    assert_eq!(rows, 0);
}

#[test]
fn deleting_the_workspace_removes_the_paths_of_its_projects() {
    let fixture = fixture();
    let store = fixture.open();
    store.save_path(fixture.project, "C:\\repo").unwrap();

    store.delete_workspace(fixture.workspace, None).unwrap();

    drop(store);
    let rows: i64 = rusqlite_count(&fixture, "SELECT COUNT(*) FROM project_repository");
    assert_eq!(rows, 0);
}

fn rusqlite_count(fixture: &Fixture, sql: &str) -> i64 {
    looji_persistence::Connection::open(fixture.db_path())
        .unwrap()
        .query_row(sql, [], |row| row.get(0))
        .unwrap()
}
