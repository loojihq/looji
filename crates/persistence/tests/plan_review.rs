//! Editing a ready plan draft. Every change is written as soon as it's applied, a refused change writes nothing, and
//! each reopen shows what the last save wrote. Each reopen is a real close and open of the database file.

use looji_domain::{
    DraftState, Estimate, MemberColour, PlanDraft, PlanDraftId, PlanDraftRepository, PlanEdit,
    PlanStory, PlanTask, Priority, Project, ProjectId, ProjectRepository, RepoError, TeamMember,
    TeamMemberId, TeamMemberRepository, Timestamp, Workspace, WorkspaceRepository,
};
use looji_persistence::SqliteStore;
use tempfile::TempDir;

struct Fixture {
    dir: TempDir,
    project: ProjectId,
    other_project: ProjectId,
    alex: TeamMemberId,
}

impl Fixture {
    fn open(&self) -> SqliteStore {
        SqliteStore::open(&self.dir.path().join("looji.db")).unwrap()
    }
}

fn at(millis: i64) -> Timestamp {
    Timestamp::from_unix_millis(millis)
}

/// A file database with two projects in one workspace, and one team member on the first.
fn fixture() -> Fixture {
    let dir = TempDir::new().unwrap();
    let store = SqliteStore::open(&dir.path().join("looji.db")).unwrap();
    let workspace = Workspace::new("Work").unwrap();
    WorkspaceRepository::save(&store, &workspace).unwrap();
    let project = Project::new(workspace.id(), "looji").unwrap();
    ProjectRepository::save(&store, &project).unwrap();
    let other = Project::new(workspace.id(), "elsewhere").unwrap();
    ProjectRepository::save(&store, &other).unwrap();
    let alex = TeamMember::new(project.id(), "Alex Kim", MemberColour::for_new_member(0)).unwrap();
    TeamMemberRepository::save(&store, &alex).unwrap();
    Fixture {
        dir,
        project: project.id(),
        other_project: other.id(),
        alex: alex.id(),
    }
}

/// A ready draft with a spec, one story, and the given tasks, saved to the store.
fn ready_draft(fx: &Fixture, store: &SqliteStore, tasks: Vec<PlanTask>) -> PlanDraft {
    let mut draft = PlanDraft::start(fx.project, "Plan the launch", at(1)).unwrap();
    draft
        .complete(
            "The spec.",
            vec![PlanStory {
                title: "Scope".to_string(),
                body: "Ship it.".to_string(),
            }],
            tasks,
            at(2),
        )
        .unwrap();
    PlanDraftRepository::save(store, &draft).unwrap();
    draft
}

fn task(title: &str) -> PlanTask {
    PlanTask::new(title).unwrap()
}

fn reload(fx: &Fixture, id: PlanDraftId) -> PlanDraft {
    PlanDraftRepository::find(&fx.open(), id)
        .unwrap()
        .expect("the draft is stored")
}

fn titles(draft: &PlanDraft) -> Vec<String> {
    draft.tasks().iter().map(|t| t.title.clone()).collect()
}

#[test]
fn an_edit_is_saved_as_soon_as_it_is_applied() {
    let fx = fixture();
    let id = {
        let store = fx.open();
        ready_draft(&fx, &store, vec![task("One")]).id()
    };

    {
        let store = fx.open();
        PlanDraftRepository::apply_edit(
            &store,
            id,
            PlanEdit::SetSpec("A sharper spec.".to_string()),
            at(10),
        )
        .unwrap();
        // Closed straight after the edit, with no save call after it.
    }

    assert_eq!(reload(&fx, id).spec(), "A sharper spec.");
    assert_eq!(reload(&fx, id).updated_at(), at(10));
}

#[test]
fn a_refused_edit_writes_nothing_and_says_why() {
    let fx = fixture();
    let (id, before) = {
        let store = fx.open();
        let draft = ready_draft(&fx, &store, vec![task("One"), task("Two")]);
        (draft.id(), draft)
    };

    let store = fx.open();
    let result = PlanDraftRepository::apply_edit(
        &store,
        id,
        PlanEdit::SetTaskTitle {
            index: 1,
            title: "ONE".to_string(),
        },
        at(10),
    );

    assert!(
        matches!(result, Err(RepoError::Rejected(_))),
        "a repeated title is refused: {result:?}"
    );
    assert_eq!(reload(&fx, id), before, "the stored draft is unchanged");
}

#[test]
fn deleting_every_task_reopens_as_a_ready_draft_with_an_empty_backlog() {
    let fx = fixture();
    let id = {
        let store = fx.open();
        ready_draft(&fx, &store, vec![task("One"), task("Two")]).id()
    };
    {
        let store = fx.open();
        PlanDraftRepository::apply_edit(&store, id, PlanEdit::RemoveTask(0), at(10)).unwrap();
        PlanDraftRepository::apply_edit(&store, id, PlanEdit::RemoveTask(0), at(11)).unwrap();
    }

    let reopened = reload(&fx, id);

    assert_eq!(reopened.state(), DraftState::Ready);
    assert!(reopened.tasks().is_empty());
    assert_eq!(reopened.spec(), "The spec.");
    assert_eq!(reopened.stories().len(), 1);
}

#[test]
fn a_task_keeps_its_priority_estimate_assignee_and_tags_across_a_reopen() {
    let fx = fixture();
    let id = {
        let store = fx.open();
        let draft = ready_draft(&fx, &store, vec![task("One")]);
        let id = draft.id();
        PlanDraftRepository::apply_edit(
            &store,
            id,
            PlanEdit::SetTaskPriority {
                index: 0,
                priority: Priority::High,
            },
            at(3),
        )
        .unwrap();
        PlanDraftRepository::apply_edit(
            &store,
            id,
            PlanEdit::SetTaskEstimate {
                index: 0,
                estimate: Some(Estimate::from_minutes(150).unwrap()),
            },
            at(4),
        )
        .unwrap();
        PlanDraftRepository::apply_edit(
            &store,
            id,
            PlanEdit::SetTaskAssignee {
                index: 0,
                assignee: Some(fx.alex),
            },
            at(5),
        )
        .unwrap();
        PlanDraftRepository::apply_edit(
            &store,
            id,
            PlanEdit::SetTaskTags {
                index: 0,
                tags: vec!["ui".to_string(), "docs".to_string()],
            },
            at(6),
        )
        .unwrap();
        id
    };

    let draft = reload(&fx, id);
    let saved = &draft.tasks()[0];

    assert_eq!(saved.priority, Priority::High);
    assert_eq!(saved.estimate.map(|e| e.minutes()), Some(150));
    assert_eq!(saved.assignee, Some(fx.alex));
    assert_eq!(saved.tags, vec!["docs", "ui"]);
}

#[test]
fn removing_the_team_member_clears_the_assignment_on_the_draft() {
    let fx = fixture();
    let id = {
        let store = fx.open();
        let mut task = task("One");
        task.assignee = Some(fx.alex);
        ready_draft(&fx, &store, vec![task]).id()
    };
    {
        let store = fx.open();
        TeamMemberRepository::remove(&store, fx.alex).unwrap();
    }

    assert_eq!(reload(&fx, id).tasks()[0].assignee, None);
}

#[test]
fn a_member_of_another_project_cannot_be_assigned() {
    let fx = fixture();
    let outsider =
        TeamMember::new(fx.other_project, "Sam Lee", MemberColour::for_new_member(0)).unwrap();
    let store = fx.open();
    TeamMemberRepository::save(&store, &outsider).unwrap();
    let draft = ready_draft(&fx, &store, vec![task("One")]);

    let result = PlanDraftRepository::apply_edit(
        &store,
        draft.id(),
        PlanEdit::SetTaskAssignee {
            index: 0,
            assignee: Some(outsider.id()),
        },
        at(10),
    );

    assert!(matches!(result, Err(RepoError::Rejected(_))), "{result:?}");
    assert_eq!(reload(&fx, draft.id()).tasks()[0].assignee, None);
}

#[test]
fn a_reordered_backlog_is_saved_in_its_new_order() {
    let fx = fixture();
    let id = {
        let store = fx.open();
        ready_draft(&fx, &store, vec![task("One"), task("Two"), task("Three")]).id()
    };
    {
        let store = fx.open();
        PlanDraftRepository::apply_edit(&store, id, PlanEdit::MoveTask { from: 2, to: 0 }, at(10))
            .unwrap();
    }

    assert_eq!(titles(&reload(&fx, id)), vec!["Three", "One", "Two"]);
}

#[test]
fn an_edit_to_a_draft_that_doesnt_exist_finds_nothing() {
    let fx = fixture();
    let store = fx.open();
    let result = PlanDraftRepository::apply_edit(
        &store,
        PlanDraftId::new(),
        PlanEdit::AddTask {
            title: "x".to_string(),
        },
        at(10),
    );
    assert_eq!(result.unwrap(), None);
}
