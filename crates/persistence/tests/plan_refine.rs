//! Refining one part of a ready plan, across reopens of the database file. The parts that aren't being refined must
//! come back exactly as they were saved, and a part edited by hand must not be replaced without the user's say-so.

use looji_domain::{
    DomainError, Estimate, MemberColour, PlanDraft, PlanDraftError, PlanDraftId,
    PlanDraftRepository, PlanEdit, PlanStory, PlanTask, Priority, Project, ProjectId,
    ProjectRepository, RefineTarget, RefinedPart, RepoError, TeamMember, TeamMemberId,
    TeamMemberRepository, Timestamp, Workspace, WorkspaceRepository,
};
use looji_persistence::SqliteStore;
use tempfile::TempDir;

struct Fixture {
    dir: TempDir,
    project: ProjectId,
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

fn fixture() -> Fixture {
    let dir = TempDir::new().unwrap();
    let store = SqliteStore::open(&dir.path().join("looji.db")).unwrap();
    let workspace = Workspace::new("Work").unwrap();
    WorkspaceRepository::save(&store, &workspace).unwrap();
    let project = Project::new(workspace.id(), "looji").unwrap();
    ProjectRepository::save(&store, &project).unwrap();
    let alex = TeamMember::new(
        project.workspace_id(),
        "Alex Kim",
        MemberColour::for_new_member(0),
    )
    .unwrap();
    TeamMemberRepository::save(&store, &alex).unwrap();
    Fixture {
        dir,
        project: project.id(),
        alex: alex.id(),
    }
}

fn story(title: &str) -> PlanStory {
    PlanStory {
        title: title.to_string(),
        body: format!("About {title}"),
        edited: false,
    }
}

fn task(title: &str, minutes: i64, assignee: Option<TeamMemberId>) -> PlanTask {
    PlanTask {
        estimate: Some(Estimate::from_minutes(minutes).unwrap()),
        priority: Priority::High,
        assignee,
        tags: vec!["ui".to_string()],
        ..PlanTask::new(title).unwrap()
    }
}

/// A ready draft with a spec, two stories, and three tasks, saved to the store.
fn ready_draft(fx: &Fixture, store: &SqliteStore) -> PlanDraft {
    let mut draft = PlanDraft::start(fx.project, "Plan the launch", at(1)).unwrap();
    draft
        .complete(
            "The spec.",
            vec![story("Scope"), story("Risks")],
            vec![
                task("One", 60, Some(fx.alex)),
                task("Two", 120, None),
                task("Three", 30, None),
            ],
            at(2),
        )
        .unwrap();
    PlanDraftRepository::save(store, &draft).unwrap();
    draft
}

fn reload(fx: &Fixture, id: PlanDraftId) -> PlanDraft {
    PlanDraftRepository::find(&fx.open(), id)
        .unwrap()
        .expect("the draft is stored")
}

#[test]
fn refining_one_task_leaves_every_other_part_exactly_as_it_was_saved() {
    let fx = fixture();
    let (before, id) = {
        let store = fx.open();
        let draft = ready_draft(&fx, &store);
        (draft.clone(), draft.id())
    };

    {
        let store = fx.open();
        PlanDraftRepository::apply_refinement(
            &store,
            id,
            RefineTarget::Task(1),
            RefinedPart::Task(task("Two", 45, None)),
            false,
            at(10),
        )
        .unwrap();
    }

    let after = reload(&fx, id);
    assert_eq!(after.spec(), before.spec());
    assert_eq!(after.stories(), before.stories());
    assert_eq!(
        after.tasks()[0],
        before.tasks()[0],
        "the first task is unchanged"
    );
    assert_eq!(
        after.tasks()[2],
        before.tasks()[2],
        "the last task is unchanged"
    );
    assert_eq!(after.tasks()[1].estimate.map(|e| e.minutes()), Some(45));
}

#[test]
fn refining_the_spec_or_a_story_leaves_the_rest_as_it_was_saved() {
    let fx = fixture();
    let (before, id) = {
        let store = fx.open();
        let draft = ready_draft(&fx, &store);
        (draft.clone(), draft.id())
    };

    {
        let store = fx.open();
        PlanDraftRepository::apply_refinement(
            &store,
            id,
            RefineTarget::Story(1),
            RefinedPart::Story(PlanStory {
                title: "Risks, narrowed".to_string(),
                body: "Fewer.".to_string(),
                edited: false,
            }),
            false,
            at(10),
        )
        .unwrap();
        PlanDraftRepository::apply_refinement(
            &store,
            id,
            RefineTarget::Spec,
            RefinedPart::Spec("A shorter spec.".to_string()),
            false,
            at(11),
        )
        .unwrap();
    }

    let after = reload(&fx, id);
    assert_eq!(after.spec(), "A shorter spec.");
    assert_eq!(
        after.stories()[0],
        before.stories()[0],
        "the other story is unchanged"
    );
    assert_eq!(after.stories()[1].title, "Risks, narrowed");
    assert_eq!(after.tasks(), before.tasks(), "the tasks are unchanged");
}

#[test]
fn a_task_edited_by_hand_is_not_replaced_until_the_user_says_so_and_nothing_is_written_meanwhile() {
    let fx = fixture();
    let id = {
        let store = fx.open();
        let draft = ready_draft(&fx, &store);
        PlanDraftRepository::apply_edit(
            &store,
            draft.id(),
            PlanEdit::SetTaskTitle {
                index: 0,
                title: "One, by hand".to_string(),
            },
            at(5),
        )
        .unwrap();
        draft.id()
    };
    let before = reload(&fx, id);
    assert!(before.tasks()[0].edited);

    {
        let store = fx.open();
        let refused = PlanDraftRepository::apply_refinement(
            &store,
            id,
            RefineTarget::Task(0),
            RefinedPart::Task(task("One", 15, None)),
            false,
            at(10),
        );
        assert!(
            matches!(
                refused,
                Err(RepoError::Rejected(DomainError::Draft(
                    PlanDraftError::ManuallyEdited
                )))
            ),
            "{refused:?}"
        );
    }
    assert_eq!(
        reload(&fx, id),
        before,
        "a refused refinement writes nothing"
    );

    {
        let store = fx.open();
        PlanDraftRepository::apply_refinement(
            &store,
            id,
            RefineTarget::Task(0),
            RefinedPart::Task(task("One", 15, None)),
            true,
            at(11),
        )
        .unwrap();
    }
    let replaced = reload(&fx, id);
    assert_eq!(replaced.tasks()[0].estimate.map(|e| e.minutes()), Some(15));
    assert!(
        !replaced.tasks()[0].edited,
        "the replacement is generated content"
    );
}

#[test]
fn a_refinement_naming_someone_outside_the_project_is_refused_and_writes_nothing() {
    let fx = fixture();
    let (before, id) = {
        let store = fx.open();
        let draft = ready_draft(&fx, &store);
        (draft.clone(), draft.id())
    };
    let outsider = TeamMemberId::new();

    let result = {
        let store = fx.open();
        PlanDraftRepository::apply_refinement(
            &store,
            id,
            RefineTarget::Task(1),
            RefinedPart::Task(PlanTask {
                assignee: Some(outsider),
                ..task("Two", 45, None)
            }),
            false,
            at(10),
        )
    };

    assert!(matches!(result, Err(RepoError::Rejected(_))), "{result:?}");
    assert_eq!(reload(&fx, id), before);
}

#[test]
fn a_refinement_of_a_draft_that_doesnt_exist_finds_nothing() {
    let fx = fixture();
    let store = fx.open();
    let result = PlanDraftRepository::apply_refinement(
        &store,
        PlanDraftId::new(),
        RefineTarget::Spec,
        RefinedPart::Spec("x".to_string()),
        false,
        at(10),
    );
    assert_eq!(result.unwrap(), None);
}
