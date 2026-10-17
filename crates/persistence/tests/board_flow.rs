//! The board end to end through the store: workspaces, projects, and tasks are created, cards are moved and reordered,
//! the store is closed and opened again, and the state is checked. Each restart is a real close and reopen of the
//! database file. The last tests have background threads, each with its own connection, writing at the same time.

use std::{
    path::PathBuf,
    sync::{Arc, Barrier},
    thread,
};

use looji_domain::{
    Position, Project, ProjectId, ProjectRepository, SettingsRepository, Status, Task, TaskId,
    TaskMove, TaskRepository, Timestamp, Workspace, WorkspaceRepository, after,
};
use looji_persistence::SqliteStore;
use tempfile::TempDir;

struct Board {
    dir: TempDir,
}

impl Board {
    fn new() -> Self {
        Self {
            dir: TempDir::new().unwrap(),
        }
    }

    fn path(&self) -> PathBuf {
        self.dir.path().join("looji.db")
    }

    /// Opens the database file. Dropping the store closes it, so calling this again is a restart.
    fn open(&self) -> SqliteStore {
        SqliteStore::open(&self.path()).unwrap()
    }
}

fn workspace(store: &SqliteStore, name: &str) -> Workspace {
    let workspace = Workspace::new(name).unwrap();
    WorkspaceRepository::save(store, &workspace).unwrap();
    workspace
}

fn project(store: &SqliteStore, workspace: &Workspace, name: &str) -> Project {
    let project = Project::new(workspace.id(), name).unwrap();
    ProjectRepository::save(store, &project).unwrap();
    project
}

/// The tasks of one column, in board order: by position, with the id as the tie-break, as the board sorts them.
fn column(store: &SqliteStore, project: ProjectId, status: Status) -> Vec<Task> {
    let mut tasks: Vec<Task> = TaskRepository::list_for_project(store, project)
        .unwrap()
        .into_iter()
        .filter(|task| task.status() == status)
        .collect();
    tasks.sort_by(|a, b| {
        a.position()
            .as_str()
            .cmp(b.position().as_str())
            .then(a.id().as_uuid().cmp(&b.id().as_uuid()))
    });
    tasks
}

fn titles(tasks: &[Task]) -> Vec<String> {
    tasks.iter().map(|task| task.title().to_string()).collect()
}

/// Adds a task at the end of a column, the way the board does: after the column's last key.
fn add_task(store: &SqliteStore, project: &Project, title: &str, status: Status) -> Task {
    let last = column(store, project.id(), status)
        .last()
        .map(|task| task.position().as_str().to_string());
    let position = Position::new(after(last.as_deref()).unwrap()).unwrap();
    let task = Task::new(project.id(), title, status, position).unwrap();
    TaskRepository::save(store, &task).unwrap();
    task
}

fn move_to(store: &SqliteStore, task: &Task, from: Status, to: Status, index: usize) {
    let mv = TaskMove::new(task.id(), task.project_id(), from, to, index, &Status::ALL).unwrap();
    store.move_task(&mv, Timestamp::now()).unwrap();
}

#[test]
fn a_board_built_and_rearranged_is_the_same_after_a_restart() {
    let board = Board::new();
    let (looji, docs, tasks) = {
        let store = board.open();
        let work = workspace(&store, "Work");
        let home = workspace(&store, "Home");
        let looji = project(&store, &work, "looji");
        let docs = project(&store, &work, "docs");
        project(&store, &home, "garden");
        let tasks: Vec<Task> = ["A", "B", "C", "D", "E"]
            .iter()
            .map(|title| add_task(&store, &looji, title, Status::Backlog))
            .collect();
        add_task(&store, &docs, "Write guide", Status::Todo);
        store.set_active_workspace(work.id()).unwrap();

        // C goes to the top of To do, A into progress, and D jumps to the front of the backlog.
        move_to(&store, &tasks[2], Status::Backlog, Status::Todo, 0);
        move_to(&store, &tasks[0], Status::Backlog, Status::InProgress, 0);
        move_to(&store, &tasks[3], Status::Backlog, Status::Backlog, 0);
        // And E goes to To do, below C.
        move_to(&store, &tasks[4], Status::Backlog, Status::Todo, 1);
        (looji, docs, tasks)
    };

    let store = board.open();

    assert_eq!(
        titles(&column(&store, looji.id(), Status::Backlog)),
        ["D", "B"]
    );
    assert_eq!(
        titles(&column(&store, looji.id(), Status::Todo)),
        ["C", "E"]
    );
    assert_eq!(
        titles(&column(&store, looji.id(), Status::InProgress)),
        ["A"]
    );
    assert_eq!(
        titles(&column(&store, docs.id(), Status::Todo)),
        ["Write guide"]
    );
    assert_eq!(WorkspaceRepository::list(&store).unwrap().len(), 2);
    assert!(store.active_workspace().unwrap().is_some());
    let history = store.task_activity(tasks[0].id()).unwrap();
    assert_eq!(history.len(), 1, "one status change was recorded for A");
    assert!(
        store.task_activity(tasks[3].id()).unwrap().is_empty(),
        "a reorder writes no history"
    );
}

#[test]
fn reordering_a_column_many_times_matches_a_model_and_keeps_positions_distinct() {
    let board = Board::new();
    let store = board.open();
    let work = workspace(&store, "Work");
    let project = project(&store, &work, "looji");
    let mut model: Vec<TaskId> = (0..20)
        .map(|n| add_task(&store, &project, &format!("task {n}"), Status::Todo).id())
        .collect();

    // A fixed sequence of moves: each takes a task from somewhere and puts it somewhere else.
    let mut seed: u64 = 7;
    for _ in 0..120 {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let from = (seed >> 33) as usize % model.len();
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let to = (seed >> 33) as usize % model.len();
        let id = model.remove(from);
        model.insert(to, id);
        let task = TaskRepository::find(&store, id).unwrap().unwrap();
        move_to(&store, &task, Status::Todo, Status::Todo, to);
    }
    drop(store);

    let store = board.open();
    let shown = column(&store, project.id(), Status::Todo);
    let ids: Vec<TaskId> = shown.iter().map(Task::id).collect();
    assert_eq!(
        ids, model,
        "the stored order is the order the moves produced"
    );
    let positions: Vec<&str> = shown.iter().map(|task| task.position().as_str()).collect();
    assert!(
        positions.windows(2).all(|pair| pair[0] < pair[1]),
        "positions are strictly increasing, so no two cards share a place: {positions:?}"
    );
}

#[test]
fn many_inserts_at_the_same_spot_keep_the_order() {
    let board = Board::new();
    let store = board.open();
    let work = workspace(&store, "Work");
    let project = project(&store, &work, "looji");
    let first = add_task(&store, &project, "first", Status::Backlog);
    let last = add_task(&store, &project, "last", Status::Backlog);

    // Each new card goes between the first card and whatever follows it, which squeezes the keys together.
    let mut model = vec![first.id(), last.id()];
    for n in 0..80 {
        let task = add_task(&store, &project, &format!("squeezed {n}"), Status::Todo);
        model.insert(1, task.id());
        move_to(&store, &task, Status::Todo, Status::Backlog, 1);
    }

    let shown = column(&store, project.id(), Status::Backlog);
    let ids: Vec<TaskId> = shown.iter().map(Task::id).collect();
    assert_eq!(ids, model);
    let positions: Vec<&str> = shown.iter().map(|task| task.position().as_str()).collect();
    assert!(
        positions.windows(2).all(|pair| pair[0] < pair[1]),
        "{positions:?}"
    );
}

#[test]
fn a_move_that_names_the_wrong_starting_status_is_refused_and_changes_nothing() {
    let board = Board::new();
    let store = board.open();
    let work = workspace(&store, "Work");
    let project = project(&store, &work, "looji");
    let task = add_task(&store, &project, "A", Status::Backlog);

    let stale = TaskMove::new(
        task.id(),
        project.id(),
        Status::Done,
        Status::Todo,
        0,
        &Status::ALL,
    )
    .unwrap();
    let result = store.move_task(&stale, Timestamp::now());

    assert!(
        result.is_err(),
        "the card isn't in Done, so the move is stale"
    );
    assert_eq!(
        titles(&column(&store, project.id(), Status::Backlog)),
        ["A"]
    );
    assert!(column(&store, project.id(), Status::Todo).is_empty());
}

#[test]
fn background_threads_moving_cards_at_once_leave_the_column_in_a_valid_order() {
    const THREADS: usize = 6;
    const MOVES_EACH: usize = 40;
    let board = Board::new();
    let (project, ids) = {
        let store = board.open();
        let work = workspace(&store, "Work");
        let project = project(&store, &work, "looji");
        let ids: Vec<TaskId> = (0..30)
            .map(|n| add_task(&store, &project, &format!("task {n}"), Status::Todo).id())
            .collect();
        (project, ids)
    };
    let (project_id, path) = (project.id(), board.path());
    let ids = Arc::new(ids);
    let start = Arc::new(Barrier::new(THREADS));

    let workers: Vec<_> = (0..THREADS)
        .map(|worker| {
            let (ids, start, path) = (Arc::clone(&ids), Arc::clone(&start), path.clone());
            thread::spawn(move || -> Vec<String> {
                // Each thread has its own connection to the same file, as separate background tasks would.
                let store = SqliteStore::open(&path).unwrap();
                let mut errors = Vec::new();
                let mut seed = 17 + worker as u64 * 1_000_003;
                start.wait();
                for _ in 0..MOVES_EACH {
                    seed = seed
                        .wrapping_mul(6364136223846793005)
                        .wrapping_add(1442695040888963407);
                    let id = ids[(seed >> 33) as usize % ids.len()];
                    seed = seed
                        .wrapping_mul(6364136223846793005)
                        .wrapping_add(1442695040888963407);
                    let index = (seed >> 33) as usize % ids.len();
                    let mv = TaskMove::new(
                        id,
                        project_id,
                        Status::Todo,
                        Status::Todo,
                        index,
                        &Status::ALL,
                    )
                    .unwrap();
                    if let Err(err) = store.move_task(&mv, Timestamp::now()) {
                        errors.push(err.to_string());
                    }
                }
                errors
            })
        })
        .collect();
    let errors: Vec<String> = workers
        .into_iter()
        .flat_map(|worker| worker.join().unwrap())
        .collect();

    assert!(
        errors.is_empty(),
        "moves failed under concurrent writes: {errors:?}"
    );
    let store = board.open();
    let shown = column(&store, project_id, Status::Todo);
    assert_eq!(shown.len(), ids.len(), "no card was lost or duplicated");
    let mut seen: Vec<TaskId> = shown.iter().map(Task::id).collect();
    seen.sort_by_key(|id| id.as_uuid());
    let mut expected: Vec<TaskId> = ids.to_vec();
    expected.sort_by_key(|id| id.as_uuid());
    assert_eq!(seen, expected, "every card is still there exactly once");
    let positions: Vec<&str> = shown.iter().map(|task| task.position().as_str()).collect();
    assert!(
        positions.windows(2).all(|pair| pair[0] < pair[1]),
        "no two cards share a position, so the order is unambiguous: {positions:?}"
    );
}

#[test]
fn background_threads_adding_and_moving_cards_at_once_lose_nothing() {
    const THREADS: usize = 4;
    const ROUNDS: usize = 15;
    let board = Board::new();
    let project = {
        let store = board.open();
        let work = workspace(&store, "Work");
        let project = project(&store, &work, "looji");
        add_task(&store, &project, "seed", Status::Todo);
        project
    };
    let (project_id, path) = (project.id(), board.path());
    let start = Arc::new(Barrier::new(THREADS));

    let workers: Vec<_> = (0..THREADS)
        .map(|worker| {
            let (start, path) = (Arc::clone(&start), path.clone());
            thread::spawn(move || -> Vec<String> {
                let store = SqliteStore::open(&path).unwrap();
                let mut errors = Vec::new();
                start.wait();
                for round in 0..ROUNDS {
                    // A new card at the end of the backlog, then straight into To do at the top.
                    // Another thread may take the place this one picked, which the store refuses. The caller picks again.
                    let mut saved = None;
                    for _ in 0..20 {
                        let last = column(&store, project_id, Status::Backlog)
                            .last()
                            .map(|task| task.position().as_str().to_string());
                        let position = Position::new(after(last.as_deref()).unwrap()).unwrap();
                        let task = Task::new(
                            project_id,
                            &format!("w{worker}-{round}"),
                            Status::Backlog,
                            position,
                        )
                        .unwrap();
                        match TaskRepository::save(&store, &task) {
                            Ok(()) => {
                                saved = Some(task);
                                break;
                            }
                            Err(err) if err.to_string().contains("UNIQUE") => continue,
                            Err(err) => {
                                errors.push(format!("save: {err}"));
                                break;
                            }
                        }
                    }
                    let Some(task) = saved else {
                        errors.push("save: never found a free place".to_string());
                        continue;
                    };
                    let mv = TaskMove::new(
                        task.id(),
                        project_id,
                        Status::Backlog,
                        Status::Todo,
                        0,
                        &Status::ALL,
                    )
                    .unwrap();
                    if let Err(err) = store.move_task(&mv, Timestamp::now()) {
                        errors.push(format!("move: {err}"));
                    }
                }
                errors
            })
        })
        .collect();
    let errors: Vec<String> = workers
        .into_iter()
        .flat_map(|worker| worker.join().unwrap())
        .collect();

    assert!(
        errors.is_empty(),
        "writes failed under concurrency: {errors:?}"
    );
    let store = board.open();
    let todo = column(&store, project_id, Status::Todo);
    assert_eq!(
        todo.len(),
        1 + THREADS * ROUNDS,
        "every card ended up in To do, once"
    );
    let positions: Vec<&str> = todo.iter().map(|task| task.position().as_str()).collect();
    assert!(
        positions.windows(2).all(|pair| pair[0] < pair[1]),
        "{positions:?}"
    );
    assert!(column(&store, project_id, Status::Backlog).is_empty());
}
