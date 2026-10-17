use looji_domain::{
    DomainError, MemberColour, Position, Project, ProjectRepository, RepoError, RepoResult, Status,
    Task, TaskEventKind, TaskRepository, TeamMember, TeamMemberRepository, Workspace,
    WorkspaceRepository,
};
use looji_persistence::SqliteStore;

fn store() -> SqliteStore {
    SqliteStore::open_in_memory().expect("in-memory store opens and migrates")
}

fn position(key: &str) -> Position {
    Position::new(key).unwrap()
}

fn workspace(store: &SqliteStore) -> Workspace {
    let workspace = Workspace::new("Work").unwrap();
    WorkspaceRepository::save(store, &workspace).unwrap();
    workspace
}

fn project(store: &SqliteStore, workspace: &Workspace, name: &str) -> Project {
    let project = Project::new(workspace.id(), name).unwrap();
    ProjectRepository::save(store, &project).unwrap();
    project
}

/// Adds a member to a project's workspace, with the colour the dialog would give it.
fn add(store: &SqliteStore, project: &Project, name: &str, existing: usize) -> TeamMember {
    let member = TeamMember::new(
        project.workspace_id(),
        name,
        MemberColour::for_new_member(existing),
    )
    .unwrap();
    TeamMemberRepository::save(store, &member).unwrap();
    member
}

fn task(store: &SqliteStore, project: &Project, title: &str) -> Task {
    // Each task gets its own position, since two tasks can't share one in the same column.
    let key = format!("a{}", uuid::Uuid::new_v4().simple());
    let task = Task::new(project.id(), title, Status::Todo, position(&key)).unwrap();
    store
        .save_task_recording(&task, &TaskEventKind::Created)
        .unwrap();
    task
}

/// Assigns a member (or nobody) through the same write the task dialog uses.
fn assign(store: &SqliteStore, task: &Task, member: Option<&TeamMember>) -> RepoResult<()> {
    let mut updated = TaskRepository::find(store, task.id()).unwrap().unwrap();
    updated.assign(member.map(TeamMember::id));
    store.save_task_recording(
        &updated,
        &TaskEventKind::Edited {
            fields: vec!["assignee".to_string()],
        },
    )
}

#[test]
fn removing_a_member_unassigns_their_tasks_and_keeps_them() {
    let store = store();
    let project = project(&store, &workspace(&store), "looji");
    let dana = add(&store, &project, "Dana", 0);
    let first = task(&store, &project, "First");
    let second = task(&store, &project, "Second");
    assign(&store, &first, Some(&dana)).unwrap();
    assign(&store, &second, Some(&dana)).unwrap();

    let unassigned = store.remove(dana.id()).unwrap();

    assert_eq!(unassigned, 2);
    assert!(
        TeamMemberRepository::find(&store, dana.id())
            .unwrap()
            .is_none()
    );
    let tasks = TaskRepository::list_for_project(&store, project.id()).unwrap();
    assert_eq!(tasks.len(), 2, "removing a member must not delete tasks");
    assert!(tasks.iter().all(|task| task.assignee_id().is_none()));
}

#[test]
fn removing_a_member_leaves_other_members_tasks_alone() {
    let store = store();
    let project = project(&store, &workspace(&store), "looji");
    let dana = add(&store, &project, "Dana", 0);
    let sam = add(&store, &project, "Sam", 1);
    let dana_task = task(&store, &project, "Dana's");
    let sam_task = task(&store, &project, "Sam's");
    assign(&store, &dana_task, Some(&dana)).unwrap();
    assign(&store, &sam_task, Some(&sam)).unwrap();

    store.remove(dana.id()).unwrap();

    let kept = TaskRepository::find(&store, sam_task.id())
        .unwrap()
        .unwrap();
    assert_eq!(kept.assignee_id(), Some(sam.id()));
    let cleared = TaskRepository::find(&store, dana_task.id())
        .unwrap()
        .unwrap();
    assert_eq!(cleared.assignee_id(), None);
    assert_eq!(cleared.title(), "Dana's");
}

#[test]
fn removing_a_member_with_no_tasks_unassigns_nothing() {
    let store = store();
    let project = project(&store, &workspace(&store), "looji");
    let dana = add(&store, &project, "Dana", 0);
    assert_eq!(store.assigned_task_count(dana.id()).unwrap(), 0);
    assert_eq!(store.remove(dana.id()).unwrap(), 0);
}

#[test]
fn the_unassignment_is_recorded_in_the_tasks_history() {
    let store = store();
    let project = project(&store, &workspace(&store), "looji");
    let dana = add(&store, &project, "Dana", 0);
    let task = task(&store, &project, "Assigned");
    assign(&store, &task, Some(&dana)).unwrap();

    store.remove(dana.id()).unwrap();

    let history = store.task_history(task.id()).unwrap();
    assert_eq!(
        history.last().unwrap().kind,
        TaskEventKind::Edited {
            fields: vec!["assignee".to_string()]
        }
    );
}

#[test]
fn renaming_keeps_the_colour_and_the_project() {
    let store = store();
    let project = project(&store, &workspace(&store), "looji");
    let mut member = add(&store, &project, "Dana", 3);

    member.rename("Dana Lee").unwrap();
    TeamMemberRepository::save(&store, &member).unwrap();

    let stored = TeamMemberRepository::find(&store, member.id())
        .unwrap()
        .unwrap();
    assert_eq!(stored.name(), "Dana Lee");
    assert_eq!(stored.colour(), MemberColour::for_new_member(3));
    assert_eq!(stored.workspace_id(), project.workspace_id());
}

#[test]
fn members_list_per_workspace_in_the_order_they_were_added() {
    let store = store();
    let workspace = workspace(&store);
    let website = project(&store, &workspace, "website");
    let docs = project(&store, &workspace, "docs");
    let first = add(&store, &website, "Sam", 0);
    let second = add(&store, &website, "Dana", 1);
    let third = add(&store, &docs, "Lee", 2);

    // One team for the whole workspace, so a member added through one project is listed for the other.
    let listed = TeamMemberRepository::list_for_workspace(&store, website.workspace_id()).unwrap();

    assert_eq!(
        listed.iter().map(TeamMember::id).collect::<Vec<_>>(),
        [first.id(), second.id(), third.id()]
    );
}

#[test]
fn a_member_of_another_project_cannot_be_assigned() {
    let store = store();
    let workspace = workspace(&store);
    let website = project(&store, &workspace, "website");
    // A member of another workspace is an outsider. A project in the same workspace shares the team.
    let elsewhere = Workspace::new("Elsewhere").unwrap();
    WorkspaceRepository::save(&store, &elsewhere).unwrap();
    let docs = project(&store, &elsewhere, "docs");
    let outsider = add(&store, &docs, "Lee", 0);
    let task = task(&store, &website, "Website copy");

    let result = assign(&store, &task, Some(&outsider));

    assert!(matches!(
        result,
        Err(RepoError::Rejected(DomainError::MemberFromOtherWorkspace))
    ));
    let stored = TaskRepository::find(&store, task.id()).unwrap().unwrap();
    assert_eq!(
        stored.assignee_id(),
        None,
        "a refused write changes nothing"
    );
}

#[test]
fn a_member_removed_before_the_save_is_refused_not_saved() {
    let store = store();
    let project = project(&store, &workspace(&store), "looji");
    let dana = add(&store, &project, "Dana", 0);
    let task = task(&store, &project, "Stale dialog");
    store.remove(dana.id()).unwrap();

    let result = assign(&store, &task, Some(&dana));

    assert!(matches!(
        result,
        Err(RepoError::Rejected(DomainError::UnknownMember))
    ));
}

#[test]
fn a_member_of_this_project_can_be_assigned() {
    let store = store();
    let project = project(&store, &workspace(&store), "looji");
    let dana = add(&store, &project, "Dana", 0);
    let task = task(&store, &project, "Assign me");

    assign(&store, &task, Some(&dana)).unwrap();

    let stored = TaskRepository::find(&store, task.id()).unwrap().unwrap();
    assert_eq!(stored.assignee_id(), Some(dana.id()));
}

#[test]
fn deleting_a_project_keeps_the_workspace_team() {
    let store = store();
    let workspace = workspace(&store);
    let website = project(&store, &workspace, "website");
    let docs = project(&store, &workspace, "docs");
    let sam = add(&store, &website, "Sam", 0);
    let lee = add(&store, &docs, "Lee", 1);

    ProjectRepository::delete(&store, website.id()).unwrap();

    // The team belongs to the workspace, so removing one of its projects leaves everyone in place.
    let listed = TeamMemberRepository::list_for_workspace(&store, workspace.id()).unwrap();
    assert_eq!(
        listed.iter().map(TeamMember::id).collect::<Vec<_>>(),
        [sam.id(), lee.id()]
    );
}

#[test]
fn the_team_and_its_assignments_survive_reopening_the_database() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("looji.db");
    let (workspace_id, member_id, task_id) = {
        let store = SqliteStore::open(&path).unwrap();
        let project = project(&store, &workspace(&store), "looji");
        let dana = add(&store, &project, "Dana", 0);
        let task = task(&store, &project, "Assigned");
        assign(&store, &task, Some(&dana)).unwrap();
        (project.workspace_id(), dana.id(), task.id())
    };

    let reopened = SqliteStore::open(&path).unwrap();

    let members = TeamMemberRepository::list_for_workspace(&reopened, workspace_id).unwrap();
    assert_eq!(members.len(), 1);
    assert_eq!(members[0].id(), member_id);
    assert_eq!(members[0].colour(), MemberColour::for_new_member(0));
    let stored = TaskRepository::find(&reopened, task_id).unwrap().unwrap();
    assert_eq!(stored.assignee_id(), Some(member_id));
}
