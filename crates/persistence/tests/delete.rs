use std::collections::BTreeSet;

use looji_domain::{
    Column, ColumnRepository, DEFAULT_WORKSPACE_NAME, Position, Project, ProjectRepository,
    SettingsRepository, Status, Task, TaskRepository, TeamMember, TeamMemberRepository, Workspace,
    WorkspaceId, WorkspaceRepository,
};
use looji_persistence::{
    Connection, DeleteError, SqliteStore, open_database, workspace_scoped_tables,
};
use tempfile::TempDir;

struct Seeded {
    workspace: Workspace,
    project_ids: Vec<String>,
    task_ids: Vec<String>,
}

fn position(key: &str) -> Position {
    Position::new(key).unwrap()
}

/// A workspace with two projects, two tasks (one assigned), a column, and a team member.
fn seed(store: &SqliteStore, name: &str) -> Seeded {
    let workspace = Workspace::new(name).unwrap();
    WorkspaceRepository::save(store, &workspace).unwrap();
    let member = TeamMember::new(workspace.id(), "Dana").unwrap();
    TeamMemberRepository::save(store, &member).unwrap();

    let mut project_ids = Vec::new();
    let mut task_ids = Vec::new();
    let mut first_project = None;
    for (index, project_name) in ["looji", "docs"].into_iter().enumerate() {
        let project = Project::new(workspace.id(), project_name).unwrap();
        ProjectRepository::save(store, &project).unwrap();
        first_project.get_or_insert(project.id());
        project_ids.push(project.id().as_uuid().to_string());

        let mut task = Task::new(project.id(), "Write it", Status::Todo, position("a0")).unwrap();
        if index == 0 {
            task.assign(Some(member.id()));
        }
        TaskRepository::save(store, &task).unwrap();
        task_ids.push(task.id().as_uuid().to_string());
    }
    let project = Project::new(workspace.id(), "extra").unwrap();
    ProjectRepository::save(store, &project).unwrap();
    project_ids.push(project.id().as_uuid().to_string());

    let column = Column::new(
        first_project.expect("the workspace has projects"),
        Status::Todo,
        "Todo",
        position("a0"),
        true,
    )
    .unwrap();
    ColumnRepository::save(store, &column).unwrap();

    Seeded {
        workspace,
        project_ids,
        task_ids,
    }
}

fn store_at(dir: &TempDir) -> (SqliteStore, std::path::PathBuf) {
    let path = dir.path().join("looji.db");
    (SqliteStore::open(&path).unwrap(), path)
}

/// Row count for every table, so a test can compare before and after.
fn snapshot(path: &std::path::Path) -> Vec<(String, i64)> {
    let connection = open_database(path).unwrap();
    let tables: Vec<String> = connection
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    tables
        .into_iter()
        .map(|table| {
            let count: i64 = connection
                .query_row(&format!("SELECT COUNT(*) FROM \"{table}\""), [], |row| {
                    row.get(0)
                })
                .unwrap();
            (table, count)
        })
        .collect()
}

fn count_where(connection: &Connection, sql: &str, id: &str) -> i64 {
    connection.query_row(sql, [id], |row| row.get(0)).unwrap()
}

#[test]
fn deleting_a_workspace_removes_every_row_it_owns() {
    let dir = TempDir::new().unwrap();
    let (store, path) = store_at(&dir);
    let doomed = seed(&store, "Doomed");
    let keeper = seed(&store, "Keeper");
    let key = doomed.workspace.id().as_uuid().to_string();

    let deleted = store
        .delete_workspace(doomed.workspace.id(), Some(keeper.workspace.id()))
        .unwrap();

    assert_eq!(deleted.active, keeper.workspace.id());
    let connection = open_database(&path).unwrap();
    assert_eq!(
        count_where(
            &connection,
            "SELECT COUNT(*) FROM workspaces WHERE id = ?1",
            &key
        ),
        0
    );
    assert_eq!(
        count_where(
            &connection,
            "SELECT COUNT(*) FROM projects WHERE workspace_id = ?1",
            &key
        ),
        0
    );
    assert_eq!(
        count_where(
            &connection,
            "SELECT COUNT(*) FROM tasks WHERE project_id IN (SELECT id FROM projects WHERE workspace_id = ?1)",
            &key
        ),
        0
    );
    assert_eq!(
        count_where(
            &connection,
            "SELECT COUNT(*) FROM team_members WHERE workspace_id = ?1",
            &key
        ),
        0
    );
    for id in &doomed.project_ids {
        assert_eq!(
            count_where(
                &connection,
                "SELECT COUNT(*) FROM projects WHERE id = ?1",
                id
            ),
            0
        );
    }
    for id in &doomed.task_ids {
        assert_eq!(
            count_where(&connection, "SELECT COUNT(*) FROM tasks WHERE id = ?1", id),
            0
        );
    }

    // The other workspace is untouched.
    let kept: i64 = connection
        .query_row("SELECT COUNT(*) FROM tasks", [], |row| row.get(0))
        .unwrap();
    assert_eq!(kept, keeper.task_ids.len() as i64);
    let violations = connection
        .prepare("PRAGMA foreign_key_check")
        .unwrap()
        .exists([])
        .unwrap();
    assert!(!violations, "foreign_key_check must report no violations");
}

#[test]
fn deleting_a_workspace_row_directly_still_cascades() {
    // The schema's ON DELETE CASCADE only works while foreign keys are on for the connection.
    let dir = TempDir::new().unwrap();
    let (store, path) = store_at(&dir);
    let seeded = seed(&store, "Doomed");
    drop(store);

    let connection = open_database(&path).unwrap();
    connection
        .execute(
            "DELETE FROM workspaces WHERE id = ?1",
            [seeded.workspace.id().as_uuid().to_string()],
        )
        .unwrap();

    let tasks: i64 = connection
        .query_row("SELECT COUNT(*) FROM tasks", [], |row| row.get(0))
        .unwrap();
    assert_eq!(tasks, 0);
}

#[test]
fn every_table_with_workspace_data_is_in_the_delete_list() {
    let dir = TempDir::new().unwrap();
    let (_store, path) = store_at(&dir);
    let connection = open_database(&path).unwrap();
    let listed: BTreeSet<&str> = workspace_scoped_tables().collect();

    let tables: Vec<String> = connection
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();

    let mut missing = Vec::new();
    for table in tables {
        if matches!(
            table.as_str(),
            "workspaces" | "settings" | "schema_migrations"
        ) {
            continue;
        }
        let columns: BTreeSet<String> = connection
            .prepare(&format!("SELECT name FROM pragma_table_info('{table}')"))
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        let scoped = columns.contains("workspace_id") || columns.contains("project_id");
        if scoped && !listed.contains(table.as_str()) {
            missing.push(table);
        }
    }
    assert!(
        missing.is_empty(),
        "tables holding workspace data are missing from WORKSPACE_SCOPED: {missing:?}"
    );
}

#[test]
fn deleting_the_last_workspace_creates_a_default_one() {
    let dir = TempDir::new().unwrap();
    let (store, _) = store_at(&dir);
    let only = seed(&store, "Only");

    let deleted = store.delete_workspace(only.workspace.id(), None).unwrap();

    let remaining = WorkspaceRepository::list(&store).unwrap();
    assert_eq!(remaining.len(), 1, "exactly one workspace must remain");
    assert_eq!(remaining[0].name(), DEFAULT_WORKSPACE_NAME);
    assert_eq!(remaining[0].id(), deleted.active);
    assert_eq!(
        SettingsRepository::active_workspace(&store).unwrap(),
        Some(deleted.active)
    );
}

#[test]
fn deleting_the_active_workspace_selects_the_given_neighbour() {
    let dir = TempDir::new().unwrap();
    let (store, _) = store_at(&dir);
    let first = seed(&store, "First");
    let second = seed(&store, "Second");

    let deleted = store
        .delete_workspace(first.workspace.id(), Some(second.workspace.id()))
        .unwrap();

    assert_eq!(deleted.active, second.workspace.id());
    assert_eq!(
        SettingsRepository::active_workspace(&store).unwrap(),
        Some(second.workspace.id())
    );
}

#[test]
fn a_stale_neighbour_falls_back_to_the_first_remaining_workspace() {
    let dir = TempDir::new().unwrap();
    let (store, _) = store_at(&dir);
    let first = seed(&store, "First");
    let second = seed(&store, "Second");

    let deleted = store
        .delete_workspace(second.workspace.id(), Some(WorkspaceId::new()))
        .unwrap();

    assert_eq!(deleted.active, first.workspace.id());
}

#[test]
fn the_selection_always_names_a_workspace_that_exists() {
    let dir = TempDir::new().unwrap();
    let (store, _) = store_at(&dir);
    let first = seed(&store, "First");
    seed(&store, "Second");

    let deleted = store.delete_workspace(first.workspace.id(), None).unwrap();

    let exists = WorkspaceRepository::list(&store)
        .unwrap()
        .iter()
        .any(|workspace| workspace.id() == deleted.active);
    assert!(exists);
}

#[test]
fn deleting_another_workspace_keeps_the_current_selection() {
    let dir = TempDir::new().unwrap();
    let (store, _) = store_at(&dir);
    let current = seed(&store, "Current");
    let other = seed(&store, "Other");

    let deleted = store
        .delete_workspace(other.workspace.id(), Some(current.workspace.id()))
        .unwrap();

    assert_eq!(deleted.active, current.workspace.id());
}

#[test]
fn a_failed_delete_rolls_back_every_change() {
    let dir = TempDir::new().unwrap();
    let (store, path) = store_at(&dir);
    let doomed = seed(&store, "Doomed");
    seed(&store, "Keeper");
    let before = snapshot(&path);

    // Fail partway through: the tasks are already deleted when this trigger fires.
    let raw = Connection::open(&path).unwrap();
    raw.execute_batch(
        "CREATE TRIGGER block_project_delete BEFORE DELETE ON projects
         BEGIN SELECT RAISE(ABORT, 'forced failure'); END;",
    )
    .unwrap();
    drop(raw);

    let result = store.delete_workspace(doomed.workspace.id(), None);

    assert!(
        matches!(result, Err(DeleteError::Storage(_))),
        "got {result:?}"
    );
    assert_eq!(
        snapshot(&path),
        before,
        "a failed delete must change nothing"
    );
}

#[test]
fn deleting_a_missing_workspace_reports_not_found() {
    let dir = TempDir::new().unwrap();
    let (store, _) = store_at(&dir);
    seed(&store, "Only");

    let result = store.delete_workspace(WorkspaceId::new(), None);

    assert!(
        matches!(result, Err(DeleteError::NotFound)),
        "got {result:?}"
    );
}

#[test]
fn the_deleted_workspace_icon_file_is_reported_for_removal() {
    let dir = TempDir::new().unwrap();
    let (store, path) = store_at(&dir);
    let doomed = seed(&store, "Doomed");
    seed(&store, "Keeper");
    let connection = Connection::open(&path).unwrap();
    connection
        .execute(
            "UPDATE workspaces SET icon_file = 'workspace-x.png' WHERE id = ?1",
            [doomed.workspace.id().as_uuid().to_string()],
        )
        .unwrap();
    drop(connection);

    let deleted = store.delete_workspace(doomed.workspace.id(), None).unwrap();

    assert_eq!(deleted.icon_files, vec!["workspace-x.png".to_string()]);
}

#[test]
fn reading_the_summary_writes_nothing() {
    let dir = TempDir::new().unwrap();
    let (store, path) = store_at(&dir);
    let seeded = seed(&store, "Doomed");
    let before = snapshot(&path);

    let summary = store.workspace_summary(seeded.workspace.id()).unwrap();

    assert_eq!(
        snapshot(&path),
        before,
        "opening the dialog must not change any row"
    );
    assert_eq!(summary.projects, 3);
    assert_eq!(summary.tasks, 2);
}
