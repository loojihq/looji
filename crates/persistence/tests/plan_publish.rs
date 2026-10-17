//! Publishing a ready plan's tasks to a project's Backlog. The count matches the draft, the draft is marked published,
//! titles the project already has are skipped, and a failure part way leaves nothing behind.

use looji_domain::{
    DomainError, DraftState, Estimate, MemberColour, PlanDraft, PlanDraftError, PlanDraftId,
    PlanDraftRepository, PlanEdit, PlanTask, Position, Priority, Project, ProjectId,
    ProjectRepository, RepoError, Status, Task, TaskRepository, TeamMember, TeamMemberId,
    TeamMemberRepository, Timestamp, Workspace, WorkspaceRepository,
};
use looji_persistence::{SqliteStore, open_database};
use tempfile::TempDir;

struct Fixture {
    dir: TempDir,
    project: ProjectId,
    alex: TeamMemberId,
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

fn planned(title: &str) -> PlanTask {
    PlanTask {
        priority: Priority::High,
        estimate: Some(Estimate::from_minutes(90).unwrap()),
        assignee: None,
        tags: vec!["docs".to_string()],
        ..PlanTask::new(title).unwrap()
    }
}

/// A ready draft with the given tasks, saved to the store.
fn ready_draft(fx: &Fixture, store: &SqliteStore, tasks: Vec<PlanTask>) -> PlanDraft {
    let mut draft = PlanDraft::start(fx.project, "Plan the launch", at(1)).unwrap();
    draft
        .complete("The spec.", Vec::new(), tasks, at(2))
        .unwrap();
    PlanDraftRepository::save(store, &draft).unwrap();
    draft
}

/// A task already in the project's Backlog.
fn existing(fx: &Fixture, store: &SqliteStore, title: &str, key: &str) {
    let task = Task::new(
        fx.project,
        title,
        Status::Backlog,
        Position::new(key).unwrap(),
    )
    .unwrap();
    TaskRepository::save(store, &task).unwrap();
}

fn backlog_titles(store: &SqliteStore, project: ProjectId) -> Vec<String> {
    TaskRepository::list_for_project(store, project)
        .unwrap()
        .iter()
        .filter(|task| task.status() == Status::Backlog)
        .map(|task| task.title().to_string())
        .collect()
}

fn reload(fx: &Fixture, id: PlanDraftId) -> PlanDraft {
    PlanDraftRepository::find(&fx.open(), id)
        .unwrap()
        .expect("the draft is stored")
}

#[test]
fn publishing_creates_a_backlog_task_for_each_draft_task_and_marks_the_draft_published() {
    let fx = fixture();
    let id = {
        let store = fx.open();
        let mut one = planned("One");
        one.assignee = Some(fx.alex);
        ready_draft(&fx, &store, vec![one, planned("Two"), planned("Three")]).id()
    };
    let draft_count = reload(&fx, id).tasks().len();

    let publication = {
        let store = fx.open();
        PlanDraftRepository::publish(&store, id, at(10))
            .unwrap()
            .unwrap()
    };

    assert_eq!(
        publication.published.len(),
        draft_count,
        "the published count matches the draft"
    );
    assert!(publication.skipped.is_empty());
    assert_eq!(
        reload(&fx, id).published_at(),
        Some(at(10)),
        "the draft is marked published"
    );

    let store = fx.open();
    let tasks = TaskRepository::list_for_project(&store, fx.project).unwrap();
    assert_eq!(tasks.len(), 3);
    let one = tasks.iter().find(|task| task.title() == "One").unwrap();
    assert_eq!(one.status(), Status::Backlog);
    assert_eq!(one.priority(), Some(Priority::High));
    assert_eq!(one.estimate().map(|e| e.minutes()), Some(90));
    assert_eq!(one.assignee_id(), Some(fx.alex));
    assert_eq!(one.tags(), &["docs".to_string()]);
}

#[test]
fn a_title_the_project_already_has_is_skipped_ignoring_case_and_the_counts_agree() {
    let fx = fixture();
    let id = {
        let store = fx.open();
        existing(&fx, &store, "two", "a0");
        ready_draft(
            &fx,
            &store,
            vec![planned("One"), planned("Two"), planned("Three")],
        )
        .id()
    };
    let draft_count = reload(&fx, id).tasks().len();

    let publication = {
        let store = fx.open();
        PlanDraftRepository::publish(&store, id, at(10))
            .unwrap()
            .unwrap()
    };

    assert_eq!(publication.skipped, vec!["Two"]);
    assert_eq!(
        publication.published.len() + publication.skipped.len(),
        draft_count,
        "every draft task is either published or skipped"
    );
    assert_eq!(
        backlog_titles(&fx.open(), fx.project),
        vec!["two", "One", "Three"],
        "the existing task is untouched, and only the new ones are added"
    );
}

#[test]
fn published_tasks_go_after_the_backlog_tasks_already_there_in_plan_order() {
    let fx = fixture();
    let id = {
        let store = fx.open();
        existing(&fx, &store, "Already here", "a0");
        ready_draft(&fx, &store, vec![planned("First"), planned("Second")]).id()
    };

    {
        let store = fx.open();
        PlanDraftRepository::publish(&store, id, at(10)).unwrap();
    }

    assert_eq!(
        backlog_titles(&fx.open(), fx.project),
        vec!["Already here", "First", "Second"]
    );
}

#[test]
fn a_failure_partway_through_rolls_back_every_insert_and_leaves_the_draft_unpublished() {
    let fx = fixture();
    let id = {
        let store = fx.open();
        existing(&fx, &store, "Already here", "a0");
        ready_draft(
            &fx,
            &store,
            vec![planned("One"), planned("Two"), planned("Three")],
        )
        .id()
    };
    // The third insert is refused by the database, after two tasks were already written in the same transaction.
    {
        let raw = open_database(&fx.path()).unwrap();
        raw.execute_batch(
            "CREATE TRIGGER refuse_three BEFORE INSERT ON tasks WHEN NEW.title = 'Three'
             BEGIN SELECT RAISE(ABORT, 'refused for the test'); END;",
        )
        .unwrap();
    }
    let before = backlog_titles(&fx.open(), fx.project);

    let store = fx.open();
    let result = PlanDraftRepository::publish(&store, id, at(10));

    assert!(matches!(result, Err(RepoError::Storage(_))), "{result:?}");
    assert_eq!(
        backlog_titles(&store, fx.project),
        before,
        "the first two tasks were rolled back with the third"
    );
    let draft = PlanDraftRepository::find(&store, id).unwrap().unwrap();
    assert_eq!(
        draft.published_at(),
        None,
        "the draft is not marked published"
    );
    assert_eq!(draft.state(), DraftState::Ready);
}

#[test]
fn publishing_twice_is_refused_and_writes_nothing_the_second_time() {
    let fx = fixture();
    let id = {
        let store = fx.open();
        ready_draft(&fx, &store, vec![planned("One")]).id()
    };
    {
        let store = fx.open();
        PlanDraftRepository::publish(&store, id, at(10)).unwrap();
    }
    let after_first = backlog_titles(&fx.open(), fx.project);

    let store = fx.open();
    let second = PlanDraftRepository::publish(&store, id, at(11));

    assert!(
        matches!(
            second,
            Err(RepoError::Rejected(DomainError::Draft(
                PlanDraftError::AlreadyPublished
            )))
        ),
        "{second:?}"
    );
    assert_eq!(backlog_titles(&store, fx.project), after_first);
}

#[test]
fn a_published_plan_refuses_edits() {
    let fx = fixture();
    let id = {
        let store = fx.open();
        ready_draft(&fx, &store, vec![planned("One")]).id()
    };
    {
        let store = fx.open();
        PlanDraftRepository::publish(&store, id, at(10)).unwrap();
    }

    let store = fx.open();
    let edit = PlanDraftRepository::apply_edit(
        &store,
        id,
        PlanEdit::AddTask {
            title: "Late".to_string(),
        },
        at(11),
    );

    assert!(matches!(edit, Err(RepoError::Rejected(_))), "{edit:?}");
    assert_eq!(reload(&fx, id).tasks().len(), 1);
}

#[test]
fn publishing_a_draft_that_doesnt_exist_finds_nothing() {
    let fx = fixture();
    let store = fx.open();
    assert_eq!(
        PlanDraftRepository::publish(&store, PlanDraftId::new(), at(10)).unwrap(),
        None
    );
}
