use looji_domain::{
    Column, ColumnRepository, DomainError, Position, Project, ProjectId, ProjectRepository,
    RepoError, Status, Task, TaskId, TaskRepository, TeamMember, TeamMemberRepository, Workspace,
    WorkspaceRepository,
};
use looji_persistence::{Connection, SqliteStore};
use tempfile::TempDir;

fn store() -> SqliteStore {
    SqliteStore::open_in_memory().expect("in-memory store opens and migrates")
}

fn position(key: &str) -> Position {
    Position::new(key).unwrap()
}

/// Saves a workspace and returns it.
fn workspace(store: &SqliteStore) -> Workspace {
    let workspace = Workspace::new("Personal").unwrap();
    WorkspaceRepository::save(store, &workspace).unwrap();
    workspace
}

fn project(store: &SqliteStore, workspace: &Workspace) -> Project {
    let project = Project::new(workspace.id(), "looji").unwrap();
    ProjectRepository::save(store, &project).unwrap();
    project
}

fn member(store: &SqliteStore, workspace: &Workspace) -> TeamMember {
    let member = TeamMember::new(workspace.id(), "Dana").unwrap();
    TeamMemberRepository::save(store, &member).unwrap();
    member
}

fn task(store: &SqliteStore, project: &Project, title: &str) -> Task {
    let task = Task::new(project.id(), title, Status::Todo, position("a0")).unwrap();
    TaskRepository::save(store, &task).unwrap();
    task
}

#[test]
fn workspace_round_trips() {
    let store = store();
    let original = Workspace::new("  Personal  ").unwrap();

    WorkspaceRepository::save(&store, &original).unwrap();

    assert_eq!(
        WorkspaceRepository::find(&store, original.id()).unwrap(),
        Some(original.clone())
    );
    assert_eq!(WorkspaceRepository::list(&store).unwrap(), vec![original]);
}

#[test]
fn renamed_workspace_saves_its_new_name() {
    let store = store();
    let mut workspace = workspace(&store);

    workspace.rename("Work").unwrap();
    WorkspaceRepository::save(&store, &workspace).unwrap();

    let loaded = WorkspaceRepository::find(&store, workspace.id())
        .unwrap()
        .unwrap();
    assert_eq!(loaded.name(), "Work");
    assert_eq!(loaded, workspace);
}

#[test]
fn project_round_trips() {
    let store = store();
    let workspace = workspace(&store);
    let original = Project::new(workspace.id(), "looji").unwrap();

    ProjectRepository::save(&store, &original).unwrap();

    assert_eq!(
        ProjectRepository::find(&store, original.id()).unwrap(),
        Some(original.clone())
    );
    assert_eq!(
        ProjectRepository::list_for_workspace(&store, workspace.id()).unwrap(),
        vec![original]
    );
}

#[test]
fn team_member_round_trips() {
    let store = store();
    let workspace = workspace(&store);
    let original = TeamMember::new(workspace.id(), "Dana").unwrap();

    TeamMemberRepository::save(&store, &original).unwrap();

    assert_eq!(
        TeamMemberRepository::find(&store, original.id()).unwrap(),
        Some(original.clone())
    );
    assert_eq!(
        TeamMemberRepository::list_for_workspace(&store, workspace.id()).unwrap(),
        vec![original]
    );
}

#[test]
fn column_round_trips() {
    let store = store();
    let workspace = workspace(&store);
    let project = project(&store, &workspace);
    let original = Column::new(
        project.id(),
        Status::InProgress,
        "In progress",
        position("a1"),
        false,
    )
    .unwrap();

    ColumnRepository::save(&store, &original).unwrap();

    assert_eq!(
        ColumnRepository::list_for_project(&store, project.id()).unwrap(),
        vec![original]
    );
}

#[test]
fn task_with_every_field_round_trips() {
    let store = store();
    let workspace = workspace(&store);
    let project = project(&store, &workspace);
    let assignee = member(&store, &workspace);

    let mut original = Task::new(
        project.id(),
        "Write tests",
        Status::InProgress,
        position("a1"),
    )
    .unwrap();
    original.set_description("Cover every repository");
    original.set_estimate_minutes(Some(90)).unwrap();
    original.assign(Some(assignee.id()));
    TaskRepository::save(&store, &original).unwrap();

    let loaded = TaskRepository::find(&store, original.id()).unwrap();
    assert_eq!(loaded, Some(original.clone()));
    assert_eq!(loaded.unwrap().estimate().map(|e| e.minutes()), Some(90));
}

#[test]
fn task_without_estimate_or_assignee_round_trips() {
    let store = store();
    let workspace = workspace(&store);
    let project = project(&store, &workspace);
    let original = task(&store, &project, "Unestimated");

    assert_eq!(
        TaskRepository::list_for_project(&store, project.id()).unwrap(),
        vec![original]
    );
}

#[test]
fn zero_estimate_round_trips_as_zero_not_none() {
    let store = store();
    let workspace = workspace(&store);
    let project = project(&store, &workspace);
    let mut original = task(&store, &project, "Quick one");
    original.set_estimate_minutes(Some(0)).unwrap();
    TaskRepository::save(&store, &original).unwrap();

    let loaded = TaskRepository::find(&store, original.id())
        .unwrap()
        .unwrap();
    assert_eq!(loaded.estimate().map(|e| e.minutes()), Some(0));
}

#[test]
fn saving_again_updates_the_stored_task() {
    let store = store();
    let workspace = workspace(&store);
    let project = project(&store, &workspace);
    let mut task = task(&store, &project, "Draft");

    task.rename("Final").unwrap();
    task.set_status(Status::Done);
    TaskRepository::save(&store, &task).unwrap();

    let loaded = TaskRepository::find(&store, task.id()).unwrap().unwrap();
    assert_eq!(loaded.title(), "Final");
    assert_eq!(loaded.status(), Status::Done);
    assert_eq!(
        TaskRepository::list_for_project(&store, project.id())
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn deleting_a_workspace_removes_its_projects_tasks_columns_and_members() {
    let store = store();
    let workspace = workspace(&store);
    let project = project(&store, &workspace);
    let member = member(&store, &workspace);
    let task = task(&store, &project, "Goes with the workspace");
    let column = Column::new(project.id(), Status::Todo, "Todo", position("a0"), true).unwrap();
    ColumnRepository::save(&store, &column).unwrap();

    WorkspaceRepository::delete(&store, workspace.id()).unwrap();

    assert!(
        WorkspaceRepository::find(&store, workspace.id())
            .unwrap()
            .is_none()
    );
    assert!(
        ProjectRepository::find(&store, project.id())
            .unwrap()
            .is_none()
    );
    assert!(TaskRepository::find(&store, task.id()).unwrap().is_none());
    assert!(
        ColumnRepository::list_for_project(&store, project.id())
            .unwrap()
            .is_empty()
    );
    assert!(
        TeamMemberRepository::find(&store, member.id())
            .unwrap()
            .is_none()
    );
}

#[test]
fn deleting_an_assignee_leaves_the_task_unassigned() {
    let store = store();
    let workspace = workspace(&store);
    let project = project(&store, &workspace);
    let member = member(&store, &workspace);
    let mut task = task(&store, &project, "Assigned");
    task.assign(Some(member.id()));
    TaskRepository::save(&store, &task).unwrap();

    TeamMemberRepository::delete(&store, member.id()).unwrap();

    let loaded = TaskRepository::find(&store, task.id()).unwrap().unwrap();
    assert_eq!(loaded.assignee_id(), None);
    assert_eq!(loaded.title(), "Assigned");
}

#[test]
fn saving_a_task_for_a_missing_project_is_rejected() {
    let store = store();
    let orphan = Task::new(ProjectId::new(), "Orphan", Status::Todo, position("a0")).unwrap();

    let result = TaskRepository::save(&store, &orphan);

    assert!(
        matches!(result, Err(RepoError::Storage(_))),
        "got {result:?}"
    );
}

#[test]
fn deleting_a_missing_row_is_not_an_error() {
    let store = store();
    assert!(TaskRepository::delete(&store, TaskId::new()).is_ok());
}

#[test]
fn stored_row_with_a_blank_title_fails_to_load() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("looji.db");
    let task_id = TaskId::new();

    // Parents go in through the store, so only the bad row is written by hand.
    let project = {
        let store = SqliteStore::open(&path).unwrap();
        let workspace = workspace(&store);
        project(&store, &workspace)
    };
    {
        let connection = Connection::open(&path).unwrap();
        connection
            .pragma_update(None, "foreign_keys", true)
            .unwrap();
        connection
            .execute(
                "INSERT INTO tasks (id, project_id, title, description, status, position,
                                    estimate_minutes, assignee_id, created_at, updated_at)
                 VALUES (?1, ?2, '   ', '', 'todo', 'a0', NULL, NULL, 0, 0)",
                [
                    task_id.as_uuid().to_string(),
                    project.id().as_uuid().to_string(),
                ],
            )
            .unwrap();
    }

    let store = SqliteStore::open(&path).unwrap();
    let result = TaskRepository::find(&store, task_id);

    assert!(
        matches!(result, Err(RepoError::Invalid(DomainError::EmptyTitle))),
        "got {result:?}"
    );
}

#[test]
fn database_rejects_a_negative_estimate_written_outside_the_domain() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("looji.db");
    let project = {
        let store = SqliteStore::open(&path).unwrap();
        let workspace = workspace(&store);
        project(&store, &workspace)
    };

    let connection = Connection::open(&path).unwrap();
    connection
        .pragma_update(None, "foreign_keys", true)
        .unwrap();
    let result = connection.execute(
        "INSERT INTO tasks (id, project_id, title, description, status, position,
                            estimate_minutes, assignee_id, created_at, updated_at)
         VALUES (?1, ?2, 'Bad', '', 'todo', 'a0', -5, NULL, 0, 0)",
        [
            TaskId::new().as_uuid().to_string(),
            project.id().as_uuid().to_string(),
        ],
    );

    assert!(result.is_err());
}
