use std::collections::BTreeMap;

use looji_domain::{
    Project, ProjectRepository, ProjectView, ProjectViewRepository, ProjectViewState, ScrollOffset,
    Workspace, WorkspaceRepository, scroll_key,
};
use looji_persistence::{SqliteStore, open_database};
use tempfile::TempDir;

fn project(store: &SqliteStore, workspace: &Workspace, name: &str) -> Project {
    let project = Project::new(workspace.id(), name).unwrap();
    ProjectRepository::save(store, &project).unwrap();
    project
}

fn workspace(store: &SqliteStore, name: &str) -> Workspace {
    let workspace = Workspace::new(name).unwrap();
    WorkspaceRepository::save(store, &workspace).unwrap();
    workspace
}

fn rows_for(path: &std::path::Path, project: looji_domain::ProjectId) -> i64 {
    let connection = open_database(path).unwrap();
    connection
        .query_row(
            "SELECT COUNT(*) FROM project_view_state WHERE project_id = ?1",
            [project.as_uuid().to_string()],
            |row| row.get(0),
        )
        .unwrap()
}

fn state(view: ProjectView) -> ProjectViewState {
    ProjectViewState {
        view,
        scroll: BTreeMap::new(),
    }
}

#[test]
fn a_project_never_saved_opens_on_the_board_and_reading_creates_no_row() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("looji.db");
    let store = SqliteStore::open(&path).unwrap();
    let home = workspace(&store, "Work");
    let website = project(&store, &home, "website");

    let loaded = store.load_view_state(website.id()).unwrap();

    assert_eq!(loaded, ProjectViewState::default());
    assert_eq!(rows_for(&path, website.id()), 0, "reading must not write");
}

#[test]
fn a_saved_view_and_scroll_round_trip_per_project() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("looji.db");
    let store = SqliteStore::open(&path).unwrap();
    let home = workspace(&store, "Work");
    let website = project(&store, &home, "website");
    let docs = project(&store, &home, "docs");
    let mut website_state = state(ProjectView::List);
    website_state.scroll.insert(
        scroll_key::LIST.to_string(),
        ScrollOffset { x: 0.0, y: 1240.5 },
    );
    store.save_view_state(website.id(), &website_state).unwrap();
    let docs_state = state(ProjectView::Roadmap);
    store.save_view_state(docs.id(), &docs_state).unwrap();

    drop(store);
    let reopened = SqliteStore::open(&path).unwrap();

    assert_eq!(
        reopened.load_view_state(website.id()).unwrap(),
        website_state
    );
    assert_eq!(reopened.load_view_state(docs.id()).unwrap(), docs_state);
}

#[test]
fn saving_again_replaces_the_previous_state() {
    let dir = TempDir::new().unwrap();
    let store = SqliteStore::open(&dir.path().join("looji.db")).unwrap();
    let home = workspace(&store, "Work");
    let website = project(&store, &home, "website");
    store
        .save_view_state(website.id(), &state(ProjectView::Workload))
        .unwrap();
    store
        .save_view_state(website.id(), &state(ProjectView::Board))
        .unwrap();

    assert_eq!(
        store.load_view_state(website.id()).unwrap().view,
        ProjectView::Board
    );
}

#[test]
fn a_damaged_scroll_value_keeps_the_view_and_drops_only_the_scroll() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("looji.db");
    let store = SqliteStore::open(&path).unwrap();
    let home = workspace(&store, "Work");
    let website = project(&store, &home, "website");
    drop(store);
    let connection = open_database(&path).unwrap();
    connection
        .execute(
            "INSERT INTO project_view_state (project_id, active_view, scroll_json, updated_at)
             VALUES (?1, 'roadmap', 'not json', 0)",
            [website.id().as_uuid().to_string()],
        )
        .unwrap();
    drop(connection);

    let store = SqliteStore::open(&path).unwrap();
    let loaded = store.load_view_state(website.id()).unwrap();

    assert_eq!(loaded.view, ProjectView::Roadmap);
    assert!(loaded.scroll.is_empty());
}

#[test]
fn an_unknown_view_name_in_the_database_is_refused_by_its_check() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("looji.db");
    let store = SqliteStore::open(&path).unwrap();
    let home = workspace(&store, "Work");
    let website = project(&store, &home, "website");
    drop(store);
    let connection = open_database(&path).unwrap();

    let inserted = connection.execute(
        "INSERT INTO project_view_state (project_id, active_view, scroll_json, updated_at)
         VALUES (?1, 'kanban', '{}', 0)",
        [website.id().as_uuid().to_string()],
    );

    assert!(inserted.is_err());
}

#[test]
fn deleting_a_project_removes_its_view_row() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("looji.db");
    let store = SqliteStore::open(&path).unwrap();
    let home = workspace(&store, "Work");
    let website = project(&store, &home, "website");
    store
        .save_view_state(website.id(), &state(ProjectView::List))
        .unwrap();
    assert_eq!(rows_for(&path, website.id()), 1);

    ProjectRepository::delete(&store, website.id()).unwrap();

    assert_eq!(rows_for(&path, website.id()), 0);
}

#[test]
fn deleting_a_workspace_leaves_no_view_rows_behind() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("looji.db");
    let store = SqliteStore::open(&path).unwrap();
    let doomed = workspace(&store, "Doomed");
    let website = project(&store, &doomed, "website");
    store
        .save_view_state(website.id(), &state(ProjectView::Roadmap))
        .unwrap();

    store.delete_workspace(doomed.id(), None).unwrap();

    let connection = open_database(&path).unwrap();
    let rows: i64 = connection
        .query_row("SELECT COUNT(*) FROM project_view_state", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(rows, 0, "no orphan rows");
}
