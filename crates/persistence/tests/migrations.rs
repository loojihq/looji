use std::path::PathBuf;

use looji_persistence::{
    Connection, Error, Migration, migrate, open_database, open_database_with_migrations,
};
use tempfile::TempDir;

const V1: Migration = Migration {
    version: 1,
    name: "create_workspaces",
    sql: "CREATE TABLE workspaces (id INTEGER PRIMARY KEY, name TEXT NOT NULL);",
};

const V2: Migration = Migration {
    version: 2,
    name: "create_projects",
    sql: "CREATE TABLE projects (
        id INTEGER PRIMARY KEY,
        workspace_id INTEGER NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
        name TEXT NOT NULL
    );",
};

const V3: Migration = Migration {
    version: 3,
    name: "add_project_color",
    sql: "ALTER TABLE projects ADD COLUMN color TEXT;",
};

/// Creates a table, then fails. The table must not survive the rollback.
const BROKEN_V2: Migration = Migration {
    version: 2,
    name: "broken_projects",
    sql: "CREATE TABLE t (id INTEGER PRIMARY KEY); INSERT INTO no_such_table VALUES (1);",
};

const ALL: &[Migration] = &[V1, V2, V3];

/// A database path inside a directory that does not exist yet.
fn fresh_path(dir: &TempDir) -> PathBuf {
    dir.path().join("looji").join("looji.db")
}

fn applied_versions(connection: &Connection) -> Vec<u32> {
    let mut statement = connection
        .prepare("SELECT version FROM schema_migrations ORDER BY version")
        .unwrap();
    statement
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<u32>>>()
        .unwrap()
}

fn table_exists(connection: &Connection, name: &str) -> bool {
    let count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
            [name],
            |row| row.get(0),
        )
        .unwrap();
    count == 1
}

fn column_exists(connection: &Connection, table: &str, column: &str) -> bool {
    let mut statement = connection
        .prepare(&format!("PRAGMA table_info({table})"))
        .unwrap();
    let names: Vec<String> = statement
        .query_map([], |row| row.get(1))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    names.iter().any(|name| name == column)
}

#[test]
fn fresh_database_reaches_latest_schema() {
    let dir = TempDir::new().unwrap();
    let path = fresh_path(&dir);

    let connection = open_database_with_migrations(&path, ALL).expect("opens a fresh database");

    assert_eq!(applied_versions(&connection), vec![1, 2, 3]);
    assert!(table_exists(&connection, "workspaces"));
    assert!(table_exists(&connection, "projects"));
    assert!(column_exists(&connection, "projects", "color"));

    let foreign_keys: i64 = connection
        .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
        .unwrap();
    assert_eq!(foreign_keys, 1, "foreign keys must be enforced");

    let journal_mode: String = connection
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .unwrap();
    assert_eq!(journal_mode, "wal");
}

#[test]
fn existing_database_upgrades_and_keeps_its_data() {
    let dir = TempDir::new().unwrap();
    let path = fresh_path(&dir);

    {
        let connection = open_database_with_migrations(&path, &[V1]).unwrap();
        connection
            .execute("INSERT INTO workspaces (name) VALUES ('Personal')", [])
            .unwrap();
    }

    let connection = open_database_with_migrations(&path, ALL).expect("upgrades an older database");

    assert_eq!(applied_versions(&connection), vec![1, 2, 3]);
    let name: String = connection
        .query_row("SELECT name FROM workspaces", [], |row| row.get(0))
        .unwrap();
    assert_eq!(name, "Personal");
    assert!(table_exists(&connection, "projects"));
}

#[test]
fn migrating_twice_applies_nothing_the_second_time() {
    let dir = TempDir::new().unwrap();
    let path = fresh_path(&dir);

    let mut connection = open_database_with_migrations(&path, ALL).unwrap();
    let applied_again = migrate(&mut connection, ALL).unwrap();

    assert!(applied_again.is_empty());
    assert_eq!(applied_versions(&connection), vec![1, 2, 3]);
}

#[test]
fn failed_migration_rolls_back_and_names_its_version() {
    let dir = TempDir::new().unwrap();
    let path = fresh_path(&dir);

    let error = open_database_with_migrations(&path, &[V1, BROKEN_V2])
        .expect_err("the broken migration must fail");
    match &error {
        Error::MigrationFailed { version, .. } => assert_eq!(*version, 2),
        other => panic!("expected MigrationFailed, got {other:?}"),
    }
    assert!(error.to_string().contains("v2"));

    let connection = Connection::open(&path).unwrap();
    assert_eq!(applied_versions(&connection), vec![1]);
    assert!(table_exists(&connection, "workspaces"));
    assert!(
        !table_exists(&connection, "t"),
        "the failed migration's table must be rolled back"
    );
    drop(connection);

    // The database is not stuck: the fixed migration applies on the next run.
    let connection = open_database_with_migrations(&path, ALL).expect("recovers after the fix");
    assert_eq!(applied_versions(&connection), vec![1, 2, 3]);
}

#[test]
fn rejects_duplicate_versions() {
    let dir = TempDir::new().unwrap();
    let path = fresh_path(&dir);

    let error = open_database_with_migrations(&path, &[V1, V1])
        .expect_err("duplicate versions must be rejected");
    assert!(
        matches!(error, Error::InvalidMigrations(_)),
        "got {error:?}"
    );
}

#[test]
fn rejects_versions_out_of_order() {
    let dir = TempDir::new().unwrap();
    let path = fresh_path(&dir);

    let error = open_database_with_migrations(&path, &[V2, V1])
        .expect_err("out-of-order versions must be rejected");
    assert!(
        matches!(error, Error::InvalidMigrations(_)),
        "got {error:?}"
    );
}

#[test]
fn rejects_database_written_by_a_newer_version() {
    let dir = TempDir::new().unwrap();
    let path = fresh_path(&dir);
    drop(open_database_with_migrations(&path, ALL).unwrap());

    let error = open_database_with_migrations(&path, &[V1, V2])
        .expect_err("a database ahead of the app must not be opened");
    assert!(
        matches!(
            error,
            Error::DatabaseAhead {
                version: 3,
                latest: 2
            }
        ),
        "got {error:?}"
    );

    // The refusal must not change the database.
    let connection = Connection::open(&path).unwrap();
    assert_eq!(applied_versions(&connection), vec![1, 2, 3]);
}

#[test]
fn production_open_creates_missing_directories_on_first_run() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("a").join("b").join("looji.db");

    let connection = open_database(&path).expect("first run succeeds");

    assert!(path.exists());
    drop(connection);
}
