//! Upgrading a database that an earlier version wrote. The database is built from the migrations up to the previous
//! schema, with data in it, so what's checked is what a real upgrade does to a real file.

use std::{fs, path::Path};

use looji_persistence::{
    Connection, Error, Migration, app_migrations, backups_dir, confirm_startup,
    open_database_with_migrations, unconfirmed_backup,
};
use tempfile::TempDir;

/// The number of migrations before the newest, which is the schema of the previous version.
fn previous_version_count() -> usize {
    app_migrations().len() - 1
}

/// A database as the previous version left it, with a project, a task, and a chat message.
fn previous_version_database(path: &Path) {
    let connection =
        open_database_with_migrations(path, &app_migrations()[..previous_version_count()]).unwrap();
    connection
        .execute_batch(
            "INSERT INTO workspaces (id, name, icon_file, created_at, updated_at)
                 VALUES ('w', 'Work', NULL, 0, 0);
             INSERT INTO projects (id, workspace_id, name, description, created_at, updated_at)
                 VALUES ('p', 'w', 'Website', NULL, 0, 0);
             INSERT INTO tasks (id, project_id, title, description, status, position,
                                estimate_minutes, assignee_id, created_at, updated_at)
                 VALUES ('t', 'p', 'Write copy', '', 'todo', 'a0', NULL, NULL, 0, 0);
             INSERT INTO task_threads (id, task_id, created_at) VALUES ('th', 't', 0);
             INSERT INTO task_messages (id, thread_id, position, role, content, model, created_at)
                 VALUES ('m', 'th', 0, 'user', 'Why this task?', NULL, 0);",
        )
        .unwrap();
}

fn highest_version(path: &Path) -> u32 {
    Connection::open(path)
        .unwrap()
        .query_row("SELECT MAX(version) FROM schema_migrations", [], |row| {
            row.get(0)
        })
        .unwrap()
}

fn backups(database: &Path) -> Vec<String> {
    match fs::read_dir(backups_dir(database)) {
        Ok(entries) => {
            let mut names: Vec<String> = entries
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            names
        }
        Err(_) => Vec::new(),
    }
}

#[test]
fn upgrading_the_previous_version_keeps_its_data_and_backs_it_up_first() {
    let dir = TempDir::new().unwrap();
    let database = dir.path().join("looji.db");
    previous_version_database(&database);

    let connection = open_database_with_migrations(&database, app_migrations()).unwrap();

    let (project, message, state): (String, String, String) = connection
        .query_row(
            "SELECT p.name, m.content, m.state FROM projects p, task_messages m",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(project, "Website");
    assert_eq!(message, "Why this task?");
    assert_eq!(state, "complete", "the new column defaults for old rows");
    assert_eq!(highest_version(&database), app_migrations().len() as u32);

    let files = backups(&database);
    assert_eq!(files.len(), 1, "{files:?}");
    let backup = backups_dir(&database).join(&files[0]);
    assert_eq!(
        highest_version(&backup),
        previous_version_count() as u32,
        "the backup is the database as it was before the upgrade"
    );
}

#[test]
fn a_failed_upgrade_puts_the_previous_database_back() {
    let dir = TempDir::new().unwrap();
    let database = dir.path().join("looji.db");
    previous_version_database(&database);
    let before = previous_version_count() as u32;

    // One migration that works, then one that fails. Both must be undone.
    let mut list: Vec<Migration> = app_migrations()[..previous_version_count()].to_vec();
    list.push(Migration {
        version: before + 1,
        name: "adds_a_column",
        sql: "ALTER TABLE tasks ADD COLUMN extra TEXT;",
    });
    list.push(Migration {
        version: before + 2,
        name: "breaks",
        sql: "INSERT INTO no_such_table VALUES (1);",
    });

    let error = open_database_with_migrations(&database, &list).unwrap_err();

    assert!(
        matches!(error, Error::UpgradeRolledBack { .. }),
        "{error:?}"
    );
    assert!(error.to_string().contains("restored"), "{error}");
    assert_eq!(
        highest_version(&database),
        before,
        "back at the previous version"
    );
    let connection = Connection::open(&database).unwrap();
    let extra: rusqlite::Result<i64> = connection.query_row(
        "SELECT COUNT(*) FROM pragma_table_info('tasks') WHERE name = 'extra'",
        [],
        |row| row.get(0),
    );
    assert_eq!(
        extra.unwrap(),
        0,
        "the migration that did work was undone too"
    );
    let title: String = connection
        .query_row("SELECT title FROM tasks WHERE id = 't'", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(title, "Write copy", "the data is intact");
    assert_eq!(
        backups(&database).len(),
        1,
        "the backup is kept after a failure"
    );
    assert!(
        !dir.path().join("looji.db-wal").exists()
            || fs::metadata(dir.path().join("looji.db-wal")).unwrap().len() == 0,
        "no log from the failed attempt is left to replay over the restored file"
    );
}

#[test]
fn a_database_that_failed_to_upgrade_can_upgrade_after_the_fix() {
    let dir = TempDir::new().unwrap();
    let database = dir.path().join("looji.db");
    previous_version_database(&database);
    let mut broken: Vec<Migration> = app_migrations()[..previous_version_count()].to_vec();
    broken.push(Migration {
        version: previous_version_count() as u32 + 1,
        name: "breaks",
        sql: "INSERT INTO no_such_table VALUES (1);",
    });
    assert!(open_database_with_migrations(&database, &broken).is_err());

    let connection = open_database_with_migrations(&database, app_migrations());

    assert!(connection.is_ok(), "{:?}", connection.err());
}

#[test]
fn the_backup_is_kept_until_the_new_version_confirms_it_started() {
    let dir = TempDir::new().unwrap();
    let database = dir.path().join("looji.db");
    previous_version_database(&database);
    // An older backup from an earlier upgrade.
    fs::create_dir_all(backups_dir(&database)).unwrap();
    fs::write(backups_dir(&database).join("looji-before-v1-1.db"), b"old").unwrap();

    drop(open_database_with_migrations(&database, app_migrations()).unwrap());

    let waiting = unconfirmed_backup(&database).expect("the upgrade is waiting to be confirmed");
    assert!(waiting.path.exists());
    assert_eq!(
        backups(&database).len(),
        2,
        "nothing is removed before the start is confirmed"
    );

    confirm_startup(&database).unwrap();

    assert!(unconfirmed_backup(&database).is_none());
    let remaining = backups(&database);
    assert_eq!(
        remaining.len(),
        1,
        "older backups are removed once the start is confirmed: {remaining:?}"
    );
    assert!(waiting.path.exists(), "the newest backup stays");
}

#[test]
fn confirming_when_no_upgrade_ran_does_nothing() {
    let dir = TempDir::new().unwrap();
    let database = dir.path().join("looji.db");
    drop(open_database_with_migrations(&database, app_migrations()).unwrap());

    confirm_startup(&database).unwrap();

    assert!(backups(&database).is_empty());
}

#[test]
fn a_first_run_has_nothing_to_back_up() {
    let dir = TempDir::new().unwrap();
    let database = dir.path().join("looji.db");

    drop(open_database_with_migrations(&database, app_migrations()).unwrap());

    assert!(backups(&database).is_empty());
    assert!(unconfirmed_backup(&database).is_none());
}

#[test]
fn opening_a_current_database_again_makes_no_backup() {
    let dir = TempDir::new().unwrap();
    let database = dir.path().join("looji.db");
    drop(open_database_with_migrations(&database, app_migrations()).unwrap());

    drop(open_database_with_migrations(&database, app_migrations()).unwrap());

    assert!(backups(&database).is_empty());
}

#[test]
fn a_database_from_a_newer_version_is_refused_and_not_backed_up() {
    let dir = TempDir::new().unwrap();
    let database = dir.path().join("looji.db");
    drop(open_database_with_migrations(&database, app_migrations()).unwrap());

    let error =
        open_database_with_migrations(&database, &app_migrations()[..previous_version_count()])
            .unwrap_err();

    assert!(matches!(error, Error::DatabaseAhead { .. }), "{error:?}");
    assert!(backups(&database).is_empty());
}
