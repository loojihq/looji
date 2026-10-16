use looji_domain::{
    Project, ProjectRepository, Workdays, WorkingSchedule, Workspace, WorkspaceRepository,
};
use looji_persistence::{SqliteStore, open_database};
use tempfile::TempDir;

fn workspace(store: &SqliteStore) -> Workspace {
    let workspace = Workspace::new("Work").unwrap();
    WorkspaceRepository::save(store, &workspace).unwrap();
    workspace
}

#[test]
fn a_new_project_is_saved_and_read_with_the_default_schedule() {
    let dir = TempDir::new().unwrap();
    let store = SqliteStore::open(&dir.path().join("looji.db")).unwrap();
    let home = workspace(&store);
    let project = Project::new(home.id(), "looji").unwrap();
    ProjectRepository::save(&store, &project).unwrap();

    let loaded = ProjectRepository::find(&store, home.id(), project.id())
        .unwrap()
        .unwrap();

    assert_eq!(loaded.working_schedule(), WorkingSchedule::default());
    assert_eq!(loaded.working_schedule().start(), 540);
    assert_eq!(loaded.working_schedule().end(), 1020);
    assert_eq!(loaded.working_schedule().workdays().mask(), 31);
}

#[test]
fn a_non_default_schedule_survives_saving_and_reopening() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("looji.db");
    let store = SqliteStore::open(&path).unwrap();
    let home = workspace(&store);
    let mut project = Project::new(home.id(), "looji").unwrap();
    let schedule =
        WorkingSchedule::try_new(8 * 60 + 30, 1440, Workdays::from_mask(0b0111_0110).unwrap())
            .unwrap();
    project.set_working_schedule(schedule);
    ProjectRepository::save(&store, &project).unwrap();
    drop(store);

    let reopened = SqliteStore::open(&path).unwrap();
    let loaded = ProjectRepository::find(&reopened, home.id(), project.id())
        .unwrap()
        .unwrap();

    assert_eq!(loaded.working_schedule(), schedule);
    assert_eq!(
        loaded.working_schedule().end(),
        1440,
        "24:00 is kept as the end of the day"
    );
}

#[test]
fn the_database_refuses_bad_schedules_written_around_the_domain() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("looji.db");
    let store = SqliteStore::open(&path).unwrap();
    let home = workspace(&store);
    let project = Project::new(home.id(), "looji").unwrap();
    ProjectRepository::save(&store, &project).unwrap();
    drop(store);
    let connection = open_database(&path).unwrap();
    let id = project.id().as_uuid().to_string();

    let bad: [(&str, &str); 6] = [
        (
            "end before start",
            "UPDATE projects SET working_start_min = 1020, working_end_min = 540 WHERE id = ?1",
        ),
        (
            "start equal to end",
            "UPDATE projects SET working_start_min = 540, working_end_min = 540 WHERE id = ?1",
        ),
        (
            "end past midnight",
            "UPDATE projects SET working_end_min = 1441 WHERE id = ?1",
        ),
        (
            "negative start",
            "UPDATE projects SET working_start_min = -1 WHERE id = ?1",
        ),
        (
            "no workdays",
            "UPDATE projects SET workdays_mask = 0 WHERE id = ?1",
        ),
        (
            "bit above sunday",
            "UPDATE projects SET workdays_mask = 128 WHERE id = ?1",
        ),
    ];
    for (name, sql) in bad {
        assert!(
            connection.execute(sql, [&id]).is_err(),
            "the database must refuse: {name}"
        );
    }
}

#[test]
fn the_sql_defaults_match_the_domain_default() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("looji.db");
    let store = SqliteStore::open(&path).unwrap();
    let home = workspace(&store);
    drop(store);
    let connection = open_database(&path).unwrap();
    // A row written without the schedule columns takes the column defaults.
    connection
        .execute(
            "INSERT INTO projects (id, workspace_id, name, description, show_in_review, created_at, updated_at)
             VALUES ('p', ?1, 'Raw', NULL, 1, 0, 0)",
            [home.id().as_uuid().to_string()],
        )
        .unwrap();
    let (start, end, mask): (i64, i64, i64) = connection
        .query_row(
            "SELECT working_start_min, working_end_min, workdays_mask FROM projects WHERE id = 'p'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();

    let default = WorkingSchedule::default();
    assert_eq!(start, i64::from(default.start()));
    assert_eq!(end, i64::from(default.end()));
    assert_eq!(mask, i64::from(default.workdays().mask()));
}
