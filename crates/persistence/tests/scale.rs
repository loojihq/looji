//! How the store behaves with a large board and a large index: 1,000 tasks and 50,000 indexed symbols.
//!
//! The timing test is ignored by default because it seeds a lot of data. Run it with
//!     cargo test -p looji-persistence --test scale -- --ignored --nocapture
//! The query-plan tests are quick and always run. They fail if a hot query stops using an index.

use std::{fs, time::Instant};

use looji_domain::{
    Position, Project, ProjectId, ProjectRepository, Status, Task, TaskMove, TaskRepository,
    Timestamp, Workspace, WorkspaceRepository, after,
};
use looji_persistence::{Connection, SqliteStore};
use tempfile::TempDir;

const TASKS: usize = 1_000;
const FILES: usize = 500;
const SYMBOLS_PER_FILE: usize = 100;

struct Big {
    dir: TempDir,
    store: SqliteStore,
    project: ProjectId,
}

fn seed() -> (Big, std::time::Duration) {
    let dir = TempDir::new().unwrap();
    let store = SqliteStore::open(&dir.path().join("looji.db")).unwrap();
    let workspace = Workspace::new("Work").unwrap();
    WorkspaceRepository::save(&store, &workspace).unwrap();
    let project = Project::new(workspace.id(), "big").unwrap();
    ProjectRepository::save(&store, &project).unwrap();

    let started = Instant::now();
    let statuses = [
        Status::Backlog,
        Status::Todo,
        Status::InProgress,
        Status::InReview,
        Status::Done,
    ];
    let mut last: Vec<Option<String>> = vec![None; statuses.len()];
    for n in 0..TASKS {
        let slot = n % statuses.len();
        let key = after(last[slot].as_deref()).unwrap();
        last[slot] = Some(key.clone());
        let mut task = Task::new(
            project.id(),
            &format!(
                "Task {n}: handle the {} flow",
                ["login", "billing", "export", "sync"][n % 4]
            ),
            statuses[slot],
            Position::new(key).unwrap(),
        )
        .unwrap();
        task.set_description("A description that is long enough to look like real text on a card.");
        task.add_tag(["web", "api", "docs"][n % 3]).unwrap();
        TaskRepository::save(&store, &task).unwrap();
    }
    let tasks_seeded = started.elapsed();

    // The index is seeded straight into its tables, since scanning 500 real files isn't what's being measured.
    let connection = Connection::open(dir.path().join("looji.db")).unwrap();
    let project_key = project.id().as_uuid().to_string();
    connection.execute_batch("BEGIN").unwrap();
    for file in 0..FILES {
        let path = format!("src/module{}/file{file}.rs", file % 20);
        connection
            .execute(
                "INSERT INTO index_files (project_id, path, hash, parser_version, skip_reason) VALUES (?1, ?2, 'h', 1, NULL)",
                (&project_key, &path),
            )
            .unwrap();
        for symbol in 0..SYMBOLS_PER_FILE {
            connection
                .execute(
                    "INSERT INTO index_symbols (project_id, path, name, kind, start_line, end_line) VALUES (?1, ?2, ?3, 'function', ?4, ?5)",
                    (&project_key, &path, format!("handle_{}_{symbol}", ["login", "billing", "export", "sync"][symbol % 4]), symbol as i64 * 2 + 1, symbol as i64 * 2 + 2),
                )
                .unwrap();
        }
        let full = dir.path().join(&path);
        fs::create_dir_all(full.parent().unwrap()).unwrap();
        fs::write(full, "fn x() {}\n".repeat(SYMBOLS_PER_FILE * 2)).unwrap();
    }
    connection.execute_batch("COMMIT").unwrap();

    let project_id = project.id();
    (
        Big {
            dir,
            store,
            project: project_id,
        },
        tasks_seeded,
    )
}

fn timed<T>(label: &str, work: impl FnOnce() -> T) -> T {
    let started = Instant::now();
    let value = work();
    eprintln!(
        "{label:<44} {:>8.1} ms",
        started.elapsed().as_secs_f64() * 1000.0
    );
    value
}

#[test]
#[ignore = "seeds a large database; run it by hand to measure"]
fn timings_with_a_thousand_tasks_and_fifty_thousand_symbols() {
    let (big, seeded) = seed();
    eprintln!(
        "{:<44} {:>8.1} ms  ({TASKS} saves, one transaction each)",
        "seed tasks",
        seeded.as_secs_f64() * 1000.0
    );

    let tasks = timed("load the board (list_for_project)", || {
        TaskRepository::list_for_project(&big.store, big.project).unwrap()
    });
    assert_eq!(tasks.len(), TASKS);

    let index = timed("load the cached index (50,000 symbols)", || {
        big.store.load_repo_index(big.project).unwrap()
    });
    assert_eq!(index.symbols.len(), FILES * SYMBOLS_PER_FILE);

    let hunks = timed("retrieve hunks for a task", || {
        big.store
            .retrieve_hunks(big.project, big.dir.path(), "Fix login handling", "", 2_000)
            .unwrap()
    });
    assert!(!hunks.is_empty());

    let mover = TaskRepository::list_for_project(&big.store, big.project).unwrap();
    let target = mover
        .iter()
        .find(|task| task.status() == Status::Todo)
        .unwrap();
    timed("move one card to the top of another column", || {
        let mv = TaskMove::new(
            target.id(),
            big.project,
            Status::Todo,
            Status::Done,
            0,
            &Status::ALL,
        )
        .unwrap();
        big.store.move_task(&mv, Timestamp::now()).unwrap();
    });

    timed("refresh the index with nothing changed (hash only)", || {
        // The files on disk are the seeded ones, which the index has no rows for, so this reads and parses them.
        // It is here to show the cost of a first scan of 500 files, not of an unchanged repository.
        let _ = big.store.refresh_repo_index(big.project, big.dir.path());
    });
}

/// The plan SQLite chose for a statement, as one line of text.
fn plan(connection: &Connection, sql: &str, params: impl rusqlite::Params) -> String {
    let mut statement = connection
        .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
        .unwrap();
    let details: Vec<String> = statement
        .query_map(params, |row| row.get::<_, String>(3))
        .unwrap()
        .map(|row| row.unwrap())
        .collect();
    details.join(" | ")
}

fn scratch() -> (TempDir, Connection) {
    let dir = TempDir::new().unwrap();
    drop(SqliteStore::open(&dir.path().join("looji.db")).unwrap());
    let connection = Connection::open(dir.path().join("looji.db")).unwrap();
    (dir, connection)
}

#[test]
fn the_hot_queries_use_an_index_and_never_scan_a_whole_table() {
    let (_dir, connection) = scratch();
    let queries: &[(&str, &str, &str)] = &[
        (
            "a project's tasks in board order",
            "SELECT id FROM tasks WHERE project_id = ?1 ORDER BY position COLLATE BINARY, id",
            "tasks",
        ),
        (
            "a column's tasks",
            "SELECT id FROM tasks WHERE project_id = ?1 AND status = 'todo' AND id != 'x' ORDER BY position",
            "tasks",
        ),
        (
            "a project's symbols",
            "SELECT path, name, kind, start_line, end_line FROM index_symbols WHERE project_id = ?1",
            "index_symbols",
        ),
        (
            "a file's symbols",
            "SELECT name FROM index_symbols WHERE project_id = ?1 AND path = 'a.rs'",
            "index_symbols",
        ),
        (
            "a project's indexed files",
            "SELECT path, hash FROM index_files WHERE project_id = ?1",
            "index_files",
        ),
        (
            "a project's tree entries",
            "SELECT path, kind FROM index_entries WHERE project_id = ?1 ORDER BY path",
            "index_entries",
        ),
        (
            "a task's history",
            "SELECT id FROM task_activity WHERE task_id = ?1 ORDER BY created_at",
            "task_activity",
        ),
        (
            "a task's events",
            "SELECT kind FROM task_events WHERE task_id = ?1 ORDER BY created_at",
            "task_events",
        ),
        (
            "a task's tags",
            "SELECT tag FROM task_tags WHERE task_id = ?1",
            "task_tags",
        ),
        (
            "a task's messages in a thread",
            "SELECT content FROM task_messages WHERE thread_id = ?1 ORDER BY position",
            "task_messages",
        ),
        (
            "a task's threads",
            "SELECT id FROM task_threads WHERE task_id = ?1",
            "task_threads",
        ),
        (
            "a member's tasks",
            "SELECT id FROM tasks WHERE assignee_id = ?1",
            "tasks",
        ),
    ];
    let mut scans = Vec::new();
    for (label, sql, table) in queries {
        let detail = plan(&connection, sql, ["x"]);
        // A line that says SCAN <table> with no index reads every row of the table.
        let scans_table = detail
            .split(" | ")
            .any(|line| line.starts_with(&format!("SCAN {table}")) && !line.contains("INDEX"));
        if scans_table {
            scans.push(format!("{label}: {detail}"));
        }
    }
    assert!(
        scans.is_empty(),
        "these queries scan a whole table:\n{}",
        scans.join("\n")
    );
}
