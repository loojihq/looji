use looji_domain::{
    Position, Project, ProjectId, ProjectRepository, Status, Task, TaskId, TaskMove,
    TaskRepository, Timestamp, Workspace, WorkspaceRepository, after,
};
use looji_persistence::SqliteStore;
use tempfile::TempDir;

fn project(store: &SqliteStore) -> Project {
    let workspace = Workspace::new("Work").unwrap();
    WorkspaceRepository::save(store, &workspace).unwrap();
    let project = Project::new(workspace.id(), "Website").unwrap();
    ProjectRepository::save(store, &project).unwrap();
    project
}

/// Saves a task at `key` in `status`. The caller picks a key that's free in the column.
fn create_at(store: &SqliteStore, project: ProjectId, status: Status, key: &str) -> Task {
    let task = Task::new(project, "Write copy", status, Position::new(key).unwrap()).unwrap();
    TaskRepository::save(store, &task).unwrap();
    task
}

/// Adds a task after the column's last key.
fn append(store: &SqliteStore, project: ProjectId, status: Status) -> Task {
    let last = column(store, project, status)
        .last()
        .map(|id| find(store, *id).position().as_str().to_string());
    let key = after(last.as_deref()).unwrap();
    create_at(store, project, status, &key)
}

/// The ids of a column's tasks, in stored order.
fn column(store: &SqliteStore, project: ProjectId, status: Status) -> Vec<TaskId> {
    TaskRepository::list_for_project(store, project)
        .unwrap()
        .into_iter()
        .filter(|task| task.status() == status)
        .map(|task| task.id())
        .collect()
}

fn find(store: &SqliteStore, task: TaskId) -> Task {
    TaskRepository::find(store, task).unwrap().unwrap()
}

/// Places `task` at `index` among its column's other tasks.
fn place(store: &SqliteStore, task: TaskId, to: Status, index: usize) {
    let task = find(store, task);
    let mv = TaskMove::new(
        task.id(),
        task.project_id(),
        task.status(),
        to,
        index,
        &Status::DEFAULTS,
    )
    .unwrap();
    store.move_task(&mv, Timestamp::now()).unwrap();
}

/// A deterministic generator, so the random tests are reproducible without a dependency.
struct Lcg(u64);

impl Lcg {
    fn next_below(&mut self, bound: usize) -> usize {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.0 >> 33) as usize) % bound
    }
}

/// The stored keys of a column are unique and strictly increasing in column order.
fn assert_keys_increase(store: &SqliteStore, project: ProjectId, status: Status) {
    let keys: Vec<String> = column(store, project, status)
        .into_iter()
        .map(|id| find(store, id).position().as_str().to_string())
        .collect();
    for pair in keys.windows(2) {
        assert!(
            pair[0] < pair[1],
            "{} must sort before {}",
            pair[0],
            pair[1]
        );
    }
}

#[test]
fn random_moves_keep_the_stored_order_equal_to_a_plain_list() {
    let store = SqliteStore::open_in_memory().unwrap();
    let website = project(&store);
    let mut model: Vec<TaskId> = (0..6)
        .map(|_| append(&store, website.id(), Status::Todo).id())
        .collect();
    let mut random = Lcg(11);

    for step in 0..1500 {
        if random.next_below(4) == 0 {
            model.push(append(&store, website.id(), Status::Todo).id());
        }
        let task = model[random.next_below(model.len())];
        let index = random.next_below(model.len());
        let from = model.iter().position(|&id| id == task).unwrap();
        model.remove(from);
        model.insert(index, task);

        place(&store, task, Status::Todo, index);

        assert_eq!(
            column(&store, website.id(), Status::Todo),
            model,
            "step {step} disagreed with the plain list"
        );
    }
    assert_keys_increase(&store, website.id(), Status::Todo);
}

#[test]
fn ten_thousand_inserts_into_one_gap_renumber_without_changing_the_order() {
    let store = SqliteStore::open_in_memory().unwrap();
    let website = project(&store);
    let top = append(&store, website.id(), Status::Todo);
    let bottom = append(&store, website.id(), Status::Todo);
    let mut model = vec![top.id(), bottom.id()];

    let mut renumbers = 0;
    let mut previous_length = find(&store, top.id()).position().as_str().len();
    for step in 0..10_000 {
        // A unique placeholder key. The task is placed right away, so the key never survives.
        let task = create_at(&store, website.id(), Status::Todo, &format!("zz{step:08}1"));
        model.insert(1, task.id());
        place(&store, task.id(), Status::Todo, 1);

        // A renumber shows as a shorter key for the inserted task.
        let length = find(&store, task.id()).position().as_str().len();
        // The task is in the gap between the top and the previous insert, so its key stays in that
        // gap. Its length can only drop after a renumber.
        if length < previous_length {
            renumbers += 1;
        }
        previous_length = length;

        if step % 500 == 0 {
            assert_eq!(
                column(&store, website.id(), Status::Todo),
                model,
                "step {step}"
            );
        }
    }

    assert_eq!(column(&store, website.id(), Status::Todo), model);
    assert_keys_increase(&store, website.id(), Status::Todo);
    assert!(renumbers > 0, "a renumber should have happened");
    let longest = column(&store, website.id(), Status::Todo)
        .into_iter()
        .map(|id| find(&store, id).position().as_str().len())
        .max()
        .unwrap();
    assert!(
        longest <= looji_domain::REBALANCE_LENGTH,
        "keys grew to {longest} characters"
    );
}

#[test]
fn moves_survive_a_restart_in_the_same_order() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("looji.db");
    let mut model: Vec<TaskId>;
    let website_id;
    {
        let store = SqliteStore::open(&path).unwrap();
        let website = project(&store);
        website_id = website.id();
        model = (0..8)
            .map(|_| append(&store, website.id(), Status::Todo).id())
            .collect();
        let mut random = Lcg(3);
        for _ in 0..300 {
            let task = model[random.next_below(model.len())];
            let index = random.next_below(model.len());
            let from = model.iter().position(|&id| id == task).unwrap();
            model.remove(from);
            model.insert(index, task);
            place(&store, task, Status::Todo, index);
        }
        assert_eq!(column(&store, website.id(), Status::Todo), model);
    }

    let store = SqliteStore::open(&path).unwrap();
    assert_eq!(column(&store, website_id, Status::Todo), model);
    drop(store);
}

#[test]
fn a_top_middle_and_bottom_insert_stay_in_place_after_a_restart() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("looji.db");
    let (website_id, expected);
    {
        let store = SqliteStore::open(&path).unwrap();
        let website = project(&store);
        website_id = website.id();
        let a = append(&store, website.id(), Status::Todo);
        let b = append(&store, website.id(), Status::Todo);
        let c = append(&store, website.id(), Status::Todo);
        // Bottom: `d` after `c`. Top: `e` before `a`. Middle: `f` between `a` and `b`.
        let d = append(&store, website.id(), Status::Todo);
        let e = append(&store, website.id(), Status::Todo);
        let f = append(&store, website.id(), Status::Todo);
        place(&store, e.id(), Status::Todo, 0);
        place(&store, f.id(), Status::Todo, 2);
        expected = vec![e.id(), a.id(), f.id(), b.id(), c.id(), d.id()];
        assert_eq!(column(&store, website.id(), Status::Todo), expected);
    }

    let store = SqliteStore::open(&path).unwrap();
    assert_eq!(column(&store, website_id, Status::Todo), expected);
    drop(store);
}
