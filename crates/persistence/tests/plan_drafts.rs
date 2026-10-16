//! Planning drafts across restarts. Each restart is a real close and reopen of the database file, so what's tested
//! is what's on disk, not what's in memory.

use looji_domain::{
    DraftState, PlanDraft, PlanDraftId, PlanDraftRepository, PlanStory, PlanTask, Project,
    ProjectId, ProjectRepository, Timestamp, Workspace, WorkspaceRepository,
};
use looji_persistence::SqliteStore;
use tempfile::TempDir;

/// A partial response, cut off mid-string. It has quotes, a newline escape, an accent, and an emoji, so a
/// damaged copy would show.
const PARTIAL_JSON: &str =
    "{\"sections\":[{\"title\":\"Scope\",\"body\":\"Ship \\\"it\\\"\\nnow — café 😀";

struct Fixture {
    dir: TempDir,
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

/// A database file with one project in it, created and closed, so tests can reopen it.
fn fixture() -> Fixture {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("looji.db");
    let store = SqliteStore::open(&path).unwrap();
    let workspace = Workspace::new("Work").unwrap();
    WorkspaceRepository::save(&store, &workspace).unwrap();
    let project = Project::new(workspace.id(), "looji").unwrap();
    ProjectRepository::save(&store, &project).unwrap();
    Fixture {
        dir,
        project: project.id(),
    }
}

fn at(millis: i64) -> Timestamp {
    Timestamp::from_unix_millis(millis)
}

fn story(title: &str) -> PlanStory {
    PlanStory {
        title: title.to_string(),
        body: format!("About {title}"),
    }
}

fn stored(store: &SqliteStore, id: PlanDraftId) -> PlanDraft {
    PlanDraftRepository::find(store, id)
        .unwrap()
        .expect("the draft is stored")
}

#[test]
fn a_completed_draft_round_trips_with_its_sections_and_tasks() {
    let fx = fixture();
    let store = fx.open();
    let mut draft = PlanDraft::start(fx.project, "Plan the launch", at(1)).unwrap();
    draft
        .complete(
            "Spec",
            vec![story("Scope"), story("Risks")],
            vec![
                PlanTask {
                    section: Some(0),
                    ..PlanTask::new("Write the brief").unwrap()
                },
                PlanTask {
                    section: None,
                    ..PlanTask::new("Rehearse the demo").unwrap()
                },
            ],
            at(2),
        )
        .unwrap();

    PlanDraftRepository::save(&store, &draft).unwrap();

    assert_eq!(stored(&store, draft.id()), draft);
}

#[test]
fn a_generating_draft_is_interrupted_on_restart_and_keeps_its_partial_text() {
    let fx = fixture();
    let id = {
        let store = fx.open();
        let mut draft = PlanDraft::start(fx.project, "Plan the launch", at(1)).unwrap();
        draft.append_response(PARTIAL_JSON, at(2)).unwrap();
        PlanDraftRepository::save(&store, &draft).unwrap();
        draft.id()
    };

    // The store is dropped here, as on a close. Opening it again is the restart.
    let restarted = fx.open();
    let draft = stored(&restarted, id);

    assert_eq!(draft.state(), DraftState::Interrupted);
    assert_eq!(
        draft.response(),
        PARTIAL_JSON,
        "the partial text is kept byte for byte"
    );
    assert_eq!(draft.stories().len(), 0, "nothing was read from it");
}

#[test]
fn only_generating_drafts_are_touched_by_a_restart() {
    let fx = fixture();
    let (ready_id, generating_id) = {
        let store = fx.open();
        let mut ready = PlanDraft::start(fx.project, "done", at(1)).unwrap();
        ready
            .complete("Spec", vec![story("Scope")], Vec::new(), at(2))
            .unwrap();
        PlanDraftRepository::save(&store, &ready).unwrap();
        let generating = PlanDraft::start(fx.project, "not done", at(3)).unwrap();
        PlanDraftRepository::save(&store, &generating).unwrap();
        (ready.id(), generating.id())
    };

    let restarted = fx.open();

    let ready = stored(&restarted, ready_id);
    assert_eq!(ready.state(), DraftState::Ready);
    assert_eq!(ready.stories().len(), 1, "a ready draft keeps its plan");
    assert_eq!(
        ready.updated_at(),
        at(2),
        "and isn't re-stamped by the restart"
    );
    assert_eq!(
        stored(&restarted, generating_id).state(),
        DraftState::Interrupted
    );
}

#[test]
fn a_failed_draft_stays_failed_across_a_restart_with_its_reason_and_text() {
    let fx = fixture();
    let id = {
        let store = fx.open();
        let mut draft = PlanDraft::start(fx.project, "Plan", at(1)).unwrap();
        draft.append_response(PARTIAL_JSON, at(2)).unwrap();
        draft.fail("the connection dropped", at(3)).unwrap();
        PlanDraftRepository::save(&store, &draft).unwrap();
        draft.id()
    };

    let draft = stored(&fx.open(), id);

    assert_eq!(
        draft.state(),
        DraftState::Failed,
        "a failure isn't an interruption"
    );
    assert_eq!(draft.failure(), Some("the connection dropped"));
    assert_eq!(draft.response(), PARTIAL_JSON);
}

#[test]
fn an_interrupted_draft_can_be_resumed_and_the_partial_text_is_kept_for_recovery() {
    let fx = fixture();
    let id = {
        let store = fx.open();
        let mut draft = PlanDraft::start(fx.project, "Plan", at(1)).unwrap();
        draft.append_response(PARTIAL_JSON, at(2)).unwrap();
        PlanDraftRepository::save(&store, &draft).unwrap();
        draft.id()
    };

    // Restart, resume, and save the new generation's state.
    {
        let store = fx.open();
        let mut draft = stored(&store, id);
        draft.resume(at(10)).unwrap();
        PlanDraftRepository::save(&store, &draft).unwrap();
    }

    // The next restart finds the resumed generation cut off too. Its recovered text is still there.
    let draft = stored(&fx.open(), id);
    assert_eq!(draft.state(), DraftState::Interrupted);
    assert_eq!(draft.recovered(), Some(PARTIAL_JSON));
    assert_eq!(draft.response(), "");
}

#[test]
fn an_interrupted_draft_can_be_discarded_and_is_gone_after_a_restart() {
    let fx = fixture();
    let id = {
        let store = fx.open();
        let draft = PlanDraft::start(fx.project, "Plan", at(1)).unwrap();
        PlanDraftRepository::save(&store, &draft).unwrap();
        draft.id()
    };
    // Reopened, so the draft is interrupted, then discarded.
    {
        let store = fx.open();
        PlanDraftRepository::delete(&store, id).unwrap();
    }

    let restarted = fx.open();

    assert_eq!(PlanDraftRepository::find(&restarted, id).unwrap(), None);
    assert!(
        PlanDraftRepository::list_for_project(&restarted, fx.project)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn drafts_list_newest_first_and_only_for_their_project() {
    let fx = fixture();
    let store = fx.open();
    let older = PlanDraft::start(fx.project, "older", at(1)).unwrap();
    let newer = PlanDraft::start(fx.project, "newer", at(2)).unwrap();
    PlanDraftRepository::save(&store, &older).unwrap();
    PlanDraftRepository::save(&store, &newer).unwrap();

    let listed = PlanDraftRepository::list_for_project(&store, fx.project).unwrap();
    let prompts: Vec<&str> = listed.iter().map(PlanDraft::prompt).collect();
    assert_eq!(prompts, vec!["newer", "older"]);

    assert!(
        PlanDraftRepository::list_for_project(&store, ProjectId::new())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn saving_again_does_not_duplicate_the_sections_or_tasks() {
    let fx = fixture();
    let store = fx.open();
    let mut draft = PlanDraft::start(fx.project, "Plan", at(1)).unwrap();
    draft
        .complete(
            "Spec",
            vec![story("A"), story("B")],
            vec![PlanTask {
                section: Some(1),
                ..PlanTask::new("one").unwrap()
            }],
            at(2),
        )
        .unwrap();
    PlanDraftRepository::save(&store, &draft).unwrap();

    // Saving the same draft again, after it was loaded back, must not add its sections or tasks a second time.
    let loaded = stored(&store, draft.id());
    PlanDraftRepository::save(&store, &loaded).unwrap();
    PlanDraftRepository::save(&store, &loaded).unwrap();

    let reloaded = stored(&store, draft.id());
    assert_eq!(reloaded.stories(), &[story("A"), story("B")]);
    assert_eq!(reloaded.tasks().len(), 1);
}

#[test]
fn deleting_the_project_deletes_its_drafts() {
    let fx = fixture();
    let store = fx.open();
    let draft = PlanDraft::start(fx.project, "Plan", at(1)).unwrap();
    PlanDraftRepository::save(&store, &draft).unwrap();

    ProjectRepository::delete(&store, fx.project).unwrap();

    assert_eq!(PlanDraftRepository::find(&store, draft.id()).unwrap(), None);
}
