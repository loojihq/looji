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
    team::TeamMember,
    workspace::Workspace,
};

/// App-wide settings. Only the active workspace is stored for now.
pub trait SettingsRepository {
    fn active_workspace(&self) -> RepoResult<Option<WorkspaceId>>;
    fn set_active_workspace(&self, id: WorkspaceId) -> RepoResult<()>;
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
    fn find(&self, id: ProjectId) -> RepoResult<Option<Project>>;
    fn list_for_workspace(&self, workspace: WorkspaceId) -> RepoResult<Vec<Project>>;
    /// Also removes the project's tasks and columns.
    fn delete(&self, id: ProjectId) -> RepoResult<()>;
}

pub trait TaskRepository {
    fn save(&self, task: &Task) -> RepoResult<()>;
    fn find(&self, id: TaskId) -> RepoResult<Option<Task>>;
    fn list_for_project(&self, project: ProjectId) -> RepoResult<Vec<Task>>;
    fn delete(&self, id: TaskId) -> RepoResult<()>;
}

pub trait ColumnRepository {
    fn save(&self, column: &Column) -> RepoResult<()>;
    fn list_for_project(&self, project: ProjectId) -> RepoResult<Vec<Column>>;
    fn delete(&self, id: ColumnId) -> RepoResult<()>;
}

pub trait TeamMemberRepository {
    fn save(&self, member: &TeamMember) -> RepoResult<()>;
    fn find(&self, id: TeamMemberId) -> RepoResult<Option<TeamMember>>;
    fn list_for_workspace(&self, workspace: WorkspaceId) -> RepoResult<Vec<TeamMember>>;
    fn delete(&self, id: TeamMemberId) -> RepoResult<()>;
}
