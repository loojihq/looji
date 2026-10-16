use looji_domain::{
    DraftIssue, DueDate, LocalMoment, Position, Project, ProjectRepository, ScheduleDraft, Status,
    Task, TaskMove, TaskRepository, Timestamp, WorkingSchedule, Workspace, WorkspaceRepository,
    preview_due_dates,
};
use looji_persistence::{ScheduleSaveError, SqliteStore};
use tempfile::TempDir;

const MON_TO_FRI: u8 = 0b0001_1111;

fn day(year: i32, month: u32, day: u32) -> DueDate {
    DueDate::new(year, month, day).unwrap()
}

/// A moment in UTC, as the app reads "now" for a move.
fn at(day: DueDate, hour: u64, minute: u64) -> Timestamp {
    let millis = day.day_number() as u64 * 86_400_000 + (hour * 60 + minute) * 60_000;
    Timestamp::from_unix_millis(millis as i64)
}

fn store() -> (TempDir, SqliteStore) {
    let dir = TempDir::new().unwrap();
    let store = SqliteStore::open(&dir.path().join("looji.db")).unwrap();
    (dir, store)
}

fn project(store: &SqliteStore) -> Project {
    let workspace = Workspace::new("Work").unwrap();
    WorkspaceRepository::save(store, &workspace).unwrap();
    let project = Project::new(workspace.id(), "looji").unwrap();
    ProjectRepository::save(store, &project).unwrap();
    project
}

fn stored_schedule(store: &SqliteStore, project: &Project) -> WorkingSchedule {
    let workspace = project.workspace_id();
    ProjectRepository::find(store, workspace, project.id())
        .unwrap()
        .unwrap()
        .working_schedule()
}

fn task(store: &SqliteStore, project: &Project, hours: u32) -> Task {
    let mut task = Task::new(
        project.id(),
        "Write copy",
        Status::Todo,
        Position::new("a0").unwrap(),
    )
    .unwrap();
    task.set_estimate_minutes(Some(i64::from(hours) * 60))
        .unwrap();
    TaskRepository::save(store, &task).unwrap();
    task
}

fn due_of(store: &SqliteStore, task: &Task) -> Option<DueDate> {
    TaskRepository::find(store, task.id())
        .unwrap()
        .unwrap()
        .due_on()
}

#[test]
fn a_valid_draft_is_saved_and_read_back() {
    let (_dir, store) = store();
    let project = project(&store);
    let draft = ScheduleDraft {
        start: 8 * 60,
        end: 16 * 60,
        workdays: 0b0000_1111,
    };

    let saved = store.save_working_schedule(project.id(), &draft).unwrap();

    assert_eq!(stored_schedule(&store, &project), saved);
    assert_eq!(saved.start(), 480);
    assert_eq!(saved.end(), 960);
    assert_eq!(saved.workdays().mask(), 0b0000_1111);
}

#[test]
fn a_draft_with_no_workdays_is_refused_and_nothing_is_written() {
    let (_dir, store) = store();
    let project = project(&store);
    let before = stored_schedule(&store, &project);
    let draft = ScheduleDraft {
        start: 8 * 60,
        end: 16 * 60,
        workdays: 0,
    };

    let result = store.save_working_schedule(project.id(), &draft);

    assert!(
        matches!(&result, Err(ScheduleSaveError::Invalid(issues)) if issues == &vec![DraftIssue::NoWorkdays]),
        "{result:?}"
    );
    assert_eq!(stored_schedule(&store, &project), before);
}

#[test]
fn a_start_not_before_the_end_is_refused_and_nothing_is_written() {
    let (_dir, store) = store();
    let project = project(&store);
    let before = stored_schedule(&store, &project);
    let draft = ScheduleDraft {
        start: 17 * 60,
        end: 9 * 60,
        workdays: MON_TO_FRI,
    };

    let result = store.save_working_schedule(project.id(), &draft);

    assert!(matches!(
        result,
        Err(ScheduleSaveError::Invalid(ref issues)) if issues == &vec![DraftIssue::StartNotBeforeEnd]
    ));
    assert_eq!(stored_schedule(&store, &project), before);
}

#[test]
fn saving_for_a_project_that_is_gone_says_so() {
    let (_dir, store) = store();
    let project = project(&store);
    let draft = ScheduleDraft::from_schedule(&WorkingSchedule::default());
    let stranger = looji_domain::ProjectId::new();
    assert!(matches!(
        store.save_working_schedule(stranger, &draft),
        Err(ScheduleSaveError::ProjectNotFound)
    ));
    assert_eq!(
        stored_schedule(&store, &project),
        WorkingSchedule::default()
    );
}

#[test]
fn saving_new_hours_does_not_change_a_task_that_already_has_a_due_date() {
    let (_dir, store) = store();
    let project = project(&store);
    let mut task = task(&store, &project, 5);
    task.set_due_on(Some(day(2026, 11, 1)));
    TaskRepository::save(&store, &task).unwrap();
    let draft = ScheduleDraft {
        start: 6 * 60,
        end: 10 * 60,
        workdays: 0b0111_1111,
    };

    store.save_working_schedule(project.id(), &draft).unwrap();

    assert_eq!(due_of(&store, &task), Some(day(2026, 11, 1)));
}

#[test]
fn the_preview_matches_the_due_date_a_real_move_stores_under_the_same_hours() {
    let (_dir, store) = store();
    let project = project(&store);
    let draft = ScheduleDraft {
        start: 8 * 60,
        end: 16 * 60,
        workdays: MON_TO_FRI,
    };
    let schedule = store.save_working_schedule(project.id(), &draft).unwrap();

    // Friday 16 October at 15:00, with five hours of work: the preview's first date is the day the
    // move stores as the due date.
    let anchor = LocalMoment::new(day(2026, 10, 16), 15 * 60);
    let predicted = preview_due_dates(&schedule, anchor, 1).unwrap()[0];

    let task = task(&store, &project, 5);
    let mv = TaskMove::new(
        task.id(),
        task.project_id(),
        Status::Todo,
        Status::InProgress,
        0,
        &Status::ALL,
    )
    .unwrap();
    store.move_task(&mv, at(day(2026, 10, 16), 15, 0)).unwrap();

    assert_eq!(due_of(&store, &task), Some(predicted));
    assert_eq!(predicted, day(2026, 10, 19));
}
