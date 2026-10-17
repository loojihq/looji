//! What storage must provide for each aggregate. The persistence crate implements these.
//!
//! The traits are synchronous. `save` is an upsert: it inserts a new entity or
//! overwrites the stored copy with the same ID. `delete` on a missing ID is not an error.

use crate::{
    board::Column,
    error::RepoResult,
    ids::{ColumnId, ProjectId, TaskId, TeamMemberId, WorkspaceId},
    project::Project,
    task::Task,
    task_filter::TaskFilter,
    task_query::{PageCursor, TaskPage, TaskQuery},
    team::TeamMember,
    workspace::Workspace,
};

/// The board filter each project remembers. It's stored per project, so opening another project shows
/// that project's filter.
pub trait ProjectFilterRepository {
    /// The filter the project last saved. Values that no longer parse are left out, and the caller
    /// prunes the rest against the project's current data.
    fn load_filter(&self, project: ProjectId) -> RepoResult<TaskFilter>;
    /// Replaces the project's saved filter. Saving the empty filter forgets the project's filter.
    fn save_filter(&self, project: ProjectId, filter: &TaskFilter) -> RepoResult<()>;
}

/// App-wide settings. Only the active workspace is stored for now.
pub trait SettingsRepository {
    fn active_workspace(&self) -> RepoResult<Option<WorkspaceId>>;
    fn set_active_workspace(&self, id: WorkspaceId) -> RepoResult<()>;
    /// The stored text for `key`, if any. Keys this version doesn't know are never asked for.
    fn get_setting(&self, key: &str) -> RepoResult<Option<String>>;
    /// Inserts or overwrites the row for `key`.
    fn set_setting(&self, key: &str, value: &str) -> RepoResult<()>;
    /// Deletes the row for `key`. Missing keys are not an error.
    fn remove_setting(&self, key: &str) -> RepoResult<()>;
}

pub trait WorkspaceRepository {
    fn save(&self, workspace: &Workspace) -> RepoResult<()>;
    fn find(&self, id: WorkspaceId) -> RepoResult<Option<Workspace>>;
    fn list(&self) -> RepoResult<Vec<Workspace>>;
    /// Also removes the workspace's projects, their tasks and columns, and its team members.
    fn delete(&self, id: WorkspaceId) -> RepoResult<()>;
}

pub trait ProjectRepository {
    fn save(&self, project: &Project) -> RepoResult<()>;
    /// Finds a project only inside `workspace`. A project from another workspace is `None`.
    fn find(&self, workspace: WorkspaceId, id: ProjectId) -> RepoResult<Option<Project>>;
    fn list_for_workspace(&self, workspace: WorkspaceId) -> RepoResult<Vec<Project>>;
    /// Also removes the project's tasks and columns.
    fn delete(&self, id: ProjectId) -> RepoResult<()>;
}

pub trait TaskRepository {
    fn save(&self, task: &Task) -> RepoResult<()>;
    fn find(&self, id: TaskId) -> RepoResult<Option<Task>>;
    fn list_for_project(&self, project: ProjectId) -> RepoResult<Vec<Task>>;
    fn delete(&self, id: TaskId) -> RepoResult<()>;
    /// The tags that tasks in `project` have right now, sorted, starting with `prefix`, and at most
    /// `limit`. It's read from the tasks themselves, so a tag that no task uses is never suggested.
    fn tags_in_use(
        &self,
        project: ProjectId,
        prefix: &str,
        limit: usize,
    ) -> RepoResult<Vec<String>>;
    /// One page of the tasks that match `query` in `project`, in position order, after `after` (or from the
    /// start). Fetches `limit` tasks, and says whether more follow. The total is counted in the same read.
    fn list_page(
        &self,
        project: ProjectId,
        query: &TaskQuery,
        after: Option<&PageCursor>,
        limit: usize,
    ) -> RepoResult<TaskPage>;
}

pub trait ColumnRepository {
    fn save(&self, column: &Column) -> RepoResult<()>;
    fn list_for_project(&self, project: ProjectId) -> RepoResult<Vec<Column>>;
    fn delete(&self, id: ColumnId) -> RepoResult<()>;
}

pub trait TeamMemberRepository {
    /// Adds a member, or saves a rename. A member's workspace and colour are fixed once it's added,
    /// so a save never changes them.
    fn save(&self, member: &TeamMember) -> RepoResult<()>;
    fn find(&self, id: TeamMemberId) -> RepoResult<Option<TeamMember>>;
    /// In the order they were added, which keeps their name suffixes stable.
    fn list_for_workspace(&self, workspace: WorkspaceId) -> RepoResult<Vec<TeamMember>>;
    /// How many tasks are assigned to the member.
    fn assigned_task_count(&self, id: TeamMemberId) -> RepoResult<u64>;
    /// Removes the member, and unassigns its tasks in the same transaction. The tasks stay. Returns
    /// how many tasks were unassigned.
    fn remove(&self, id: TeamMemberId) -> RepoResult<u64>;
}

/// The list settings each project remembers: the sort, and the columns shown. A project that has never
/// saved them gets the defaults.
pub trait ListViewRepository {
    fn load_list_view(&self, project: ProjectId) -> RepoResult<crate::list_view::ListView>;
    fn save_list_view(
        &self,
        project: ProjectId,
        view: &crate::list_view::ListView,
    ) -> RepoResult<()>;
}

/// Which view a project shows and where it was scrolled. A project with no saved row reads as the default.
pub trait ProjectViewRepository {
    fn load_view_state(
        &self,
        project: ProjectId,
    ) -> RepoResult<crate::project_view::ProjectViewState>;
    fn save_view_state(
        &self,
        project: ProjectId,
        state: &crate::project_view::ProjectViewState,
    ) -> RepoResult<()>;
}

/// The local repository folder a project is connected to. Only the stored path lives here; whether the folder
/// still exists is checked on disk when the project loads, so a missing folder never clears the path.
pub trait CodeSourceRepository {
    /// The stored path, or `None` if the project isn't connected.
    fn load_path(&self, project: ProjectId) -> RepoResult<Option<String>>;
    /// Stores `path` as the project's repository, replacing any earlier one. The caller has already trimmed it
    /// and checked that it's a git repository.
    fn save_path(&self, project: ProjectId, path: &str) -> RepoResult<()>;
    /// Removes the stored path. Only the user's Disconnect does this.
    fn clear_path(&self, project: ProjectId) -> RepoResult<()>;
}

/// Planning drafts for a project. A draft is saved whole: its text, state, sections, and tasks go in one
/// transaction, so a reopened draft matches the last save.
pub trait PlanDraftRepository {
    fn save(&self, draft: &crate::plan_draft::PlanDraft) -> RepoResult<()>;
    fn find(&self, id: crate::ids::PlanDraftId)
    -> RepoResult<Option<crate::plan_draft::PlanDraft>>;
    /// Newest first.
    fn list_for_project(&self, project: ProjectId)
    -> RepoResult<Vec<crate::plan_draft::PlanDraft>>;
    /// Deletes a draft and its stories and tasks. This is how a draft is discarded.
    fn delete(&self, id: crate::ids::PlanDraftId) -> RepoResult<()>;
    /// Publishes a ready plan's tasks into the project's Backlog or To do column (`status`), in one transaction,
    /// skipping titles the project already has. The draft is marked published in the same transaction. A failure
    /// part way writes nothing. `None` means there is no such draft.
    fn publish(
        &self,
        id: crate::ids::PlanDraftId,
        at: crate::time::Timestamp,
        status: crate::status::Status,
    ) -> RepoResult<Option<crate::plan_draft::Publication>>;
    /// Applies one change to a ready draft and saves it. The change is checked against the project's current team.
    /// A refused change is `Rejected` and writes nothing. `None` means there is no such draft.
    fn apply_edit(
        &self,
        id: crate::ids::PlanDraftId,
        edit: crate::plan_draft::PlanEdit,
        at: crate::time::Timestamp,
    ) -> RepoResult<Option<crate::plan_draft::PlanDraft>>;
    /// Replaces one part of a ready plan and saves it. A part a person edited by hand is refused unless
    /// `replace_edited` is set. A refused change writes nothing. `None` means there is no such draft.
    fn apply_refinement(
        &self,
        id: crate::ids::PlanDraftId,
        target: crate::plan_draft::RefineTarget,
        part: crate::plan_draft::RefinedPart,
        replace_edited: bool,
        at: crate::time::Timestamp,
    ) -> RepoResult<Option<crate::plan_draft::PlanDraft>>;
}

/// The conversations held about a task. A thread belongs to one task, and its messages are kept in the order they
/// were written.
pub trait TaskChatRepository {
    /// Starts an empty thread on the task.
    fn start_thread(&self, task: TaskId) -> RepoResult<crate::ids::ChatThreadId>;
    /// The task's threads, oldest first.
    fn threads(&self, task: TaskId) -> RepoResult<Vec<crate::ids::ChatThreadId>>;
    /// Adds a message to the end of the thread, and returns its position. `model` is set for assistant messages.
    fn append_message(
        &self,
        thread: crate::ids::ChatThreadId,
        role: crate::task_chat::MessageRole,
        content: &str,
        model: Option<&str>,
    ) -> RepoResult<u32>;
    /// Sets the final text of the message at `position`, and how the reply ended.
    fn finish_message(
        &self,
        thread: crate::ids::ChatThreadId,
        position: u32,
        content: &str,
        state: &crate::chat::ReplyState,
    ) -> RepoResult<()>;
    /// Removes the message at `position` and every message after it.
    fn truncate_from(&self, thread: crate::ids::ChatThreadId, position: u32) -> RepoResult<()>;
    /// The thread's messages, in the order they were written.
    fn messages(
        &self,
        thread: crate::ids::ChatThreadId,
    ) -> RepoResult<Vec<crate::task_chat::ChatMessage>>;
}

/// The review notes made for a task. Making notes again replaces the earlier ones.
pub trait ReviewNoteRepository {
    fn replace_notes(
        &self,
        task: TaskId,
        notes: &[crate::review_note::ReviewNote],
        model: Option<&str>,
    ) -> RepoResult<()>;
    fn notes(&self, task: TaskId) -> RepoResult<Vec<crate::review_note::ReviewNote>>;
}
