//! Planning drafts as a project's list: what it shows, what resuming a draft does across a restart, that a published
//! plan can't be resumed or changed, and that a deleted project's drafts don't appear anywhere.

use looji_domain::{
    DraftState, PlanDraft, PlanDraftId, PlanDraftRepository, PlanEdit, PlanTask, Project,
    ProjectId, ProjectRepository, Timestamp, Workspace, WorkspaceId, WorkspaceRepository,
};
use looji_persistence::{SqliteStore, open_database};
use tempfile::TempDir;

struct Fixture {
    dir: TempDir,
    workspace: WorkspaceId,
    project: ProjectId,
}

impl Fixture {
    fn path(&self) -> std::path::PathBuf {
        self.dir.path().join("looji.db")
    }

    fn open(&self) -> SqliteStore {
        SqliteStore::open(&self.path()).unwrap()
    }
}

fn at(millis: i64) -> Timestamp {
    Timestamp::from_unix_millis(millis)
}

fn fixture() -> Fixture {
    let dir = TempDir::new().unwrap();
    let store = SqliteStore::open(&dir.path().join("looji.db")).unwrap();
    let workspace = Workspace::new("Work").unwrap();
    WorkspaceRepository::save(&store, &workspace).unwrap();
    let project = Project::new(workspace.id(), "looji").unwrap();
    ProjectRepository::save(&store, &project).unwrap();
    Fixture {
        dir,
        workspace: workspace.id(),
        project: project.id(),
    }
}

fn ready(fx: &Fixture, prompt: &str, created: i64) -> PlanDraft {
    let mut draft = PlanDraft::start(fx.project, prompt, at(created)).unwrap();
    draft
        .complete(
            "Spec for the plan.",
            Vec::new(),
            vec![PlanTask::new("One").unwrap()],
            at(created + 1),
        )
        .unwrap();
    draft
}

#[test]
fn a_project_lists_its_drafts_with_their_states_and_times() {
    let fx = fixture();
    let store = fx.open();
    let generating = PlanDraft::start(fx.project, "Still generating", at(1)).unwrap();
    let plan = ready(&fx, "A finished plan", 2);
    PlanDraftRepository::save(&store, &generating).unwrap();
    PlanDraftRepository::save(&store, &plan).unwrap();

    let listed = PlanDraftRepository::list_for_project(&store, fx.project).unwrap();

    assert_eq!(listed.len(), 2);
    let finished = listed.iter().find(|d| d.id() == plan.id()).unwrap();
    assert_eq!(finished.state(), DraftState::Ready);
    assert_eq!(finished.title(), "Spec for the plan.");
    assert_eq!(
        finished.updated_at(),
        at(3),
        "the list shows when it was last updated"
    );
}

#[test]
fn a_draft_interrupted_by_a_restart_can_be_resumed_after_it() {
    let fx = fixture();
    let id = {
        let store = fx.open();
        let mut draft = PlanDraft::start(fx.project, "Plan the launch", at(1)).unwrap();
        draft.append_response("partial", at(2)).unwrap();
        PlanDraftRepository::save(&store, &draft).unwrap();
        draft.id()
    };

    // The restart interrupts it. Resuming it is the list's Resume action.
    let store = fx.open();
    let mut draft = PlanDraftRepository::find(&store, id).unwrap().unwrap();
    assert_eq!(draft.state(), DraftState::Interrupted);
    draft.resume(at(10)).unwrap();
    PlanDraftRepository::save(&store, &draft).unwrap();

    let resumed = PlanDraftRepository::find(&fx.open(), id).unwrap().unwrap();
    assert_eq!(
        resumed.state(),
        DraftState::Interrupted,
        "a restart after the resume interrupts it again"
    );
    assert_eq!(
        resumed.recovered(),
        Some("partial"),
        "the old text is kept for recovery"
    );
}

#[test]
fn a_published_plan_is_read_only_it_cannot_be_resumed_or_changed() {
    let fx = fixture();
    let store = fx.open();
    let plan = ready(&fx, "Published", 1);
    PlanDraftRepository::save(&store, &plan).unwrap();
    PlanDraftRepository::publish(&store, plan.id(), at(10)).unwrap();

    let mut published = PlanDraftRepository::find(&store, plan.id())
        .unwrap()
        .unwrap();
    assert!(published.published_at().is_some());
    assert!(
        published.resume(at(11)).is_err(),
        "a published plan can't be resumed"
    );
    assert!(
        published
            .apply(
                PlanEdit::AddTask {
                    title: "Late".to_string()
                },
                &[],
                at(12)
            )
            .is_err(),
        "a published plan can't be edited"
    );
    assert_eq!(
        PlanDraftRepository::find(&store, plan.id())
            .unwrap()
            .unwrap()
            .tasks()
            .len(),
        1,
        "and nothing about it changed in the store"
    );
}

#[test]
fn a_duplicate_of_a_published_plan_is_a_new_unpublished_plan() {
    let fx = fixture();
    let store = fx.open();
    let plan = ready(&fx, "Published", 1);
    PlanDraftRepository::save(&store, &plan).unwrap();
    PlanDraftRepository::publish(&store, plan.id(), at(10)).unwrap();
    let original = PlanDraftRepository::find(&store, plan.id())
        .unwrap()
        .unwrap();

    let copy = original.duplicate(PlanDraftId::new(), at(11)).unwrap();
    PlanDraftRepository::save(&store, &copy).unwrap();

    let stored = PlanDraftRepository::find(&store, copy.id())
        .unwrap()
        .unwrap();
    assert_eq!(stored.published_at(), None);
    assert_eq!(stored.tasks().len(), 1);
    assert_eq!(
        PlanDraftRepository::find(&store, plan.id())
            .unwrap()
            .unwrap()
            .published_at(),
        Some(at(10)),
        "the original is still published"
    );
}

#[test]
fn deleting_a_draft_removes_it_from_the_list() {
    let fx = fixture();
    let store = fx.open();
    let draft = ready(&fx, "Delete me", 1);
    PlanDraftRepository::save(&store, &draft).unwrap();

    PlanDraftRepository::delete(&store, draft.id()).unwrap();

    assert!(
        PlanDraftRepository::list_for_project(&store, fx.project)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn a_deleted_projects_drafts_do_not_appear_anywhere() {
    let fx = fixture();
    let store = fx.open();
    let draft = ready(&fx, "Belongs to a project that goes away", 1);
    PlanDraftRepository::save(&store, &draft).unwrap();

    ProjectRepository::delete(&store, fx.project).unwrap();

    assert!(
        PlanDraftRepository::list_for_project(&store, fx.project)
            .unwrap()
            .is_empty()
    );
    assert_eq!(PlanDraftRepository::find(&store, draft.id()).unwrap(), None);
    let rows: i64 = open_database(&fx.path())
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM plan_drafts WHERE project_id = ?1",
            [fx.project.as_uuid().to_string()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(rows, 0, "no row is left behind for the deleted project");
}

#[test]
fn deleting_the_workspace_removes_its_projects_drafts_too() {
    let fx = fixture();
    let store = fx.open();
    let draft = ready(&fx, "Belongs to a workspace that goes away", 1);
    PlanDraftRepository::save(&store, &draft).unwrap();

    store.delete_workspace(fx.workspace, None).unwrap();

    assert_eq!(PlanDraftRepository::find(&store, draft.id()).unwrap(), None);
    assert!(
        PlanDraftRepository::list_for_project(&store, fx.project)
            .unwrap()
            .is_empty()
    );
}
