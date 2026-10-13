//! Plain Rust domain types and rules.
//!
//! This crate must not depend on GPUI or SQLite. Storage is described by the
//! repository traits in [`repository`]; the persistence crate implements them.

mod board;
mod error;
mod ids;
mod position;
pub mod preferences;
mod project;
pub mod repository;
mod status;
mod status_columns;
mod task;
mod team;
mod time;
mod validation;
mod workspace;

pub use board::{Column, ColumnRecord};
pub use error::{DomainError, RepoError, RepoResult};
pub use ids::{ColumnId, ProjectId, TaskId, TeamMemberId, WorkspaceId};
pub use position::Position;
pub use preferences::{
    DefaultView, Preference, Preferences, Theme as PreferredTheme, UpdateChannel,
};
pub use project::{
    MAX_DESCRIPTION_CHARS, Project, ProjectRecord, clean_description, validate_project_name,
};
pub use repository::{
    ColumnRepository, ProjectRepository, SettingsRepository, TaskRepository, TeamMemberRepository,
    WorkspaceRepository,
};
pub use status::Status;
pub use status_columns::{StatusColumn, status_columns};
pub use task::{Estimate, Task, TaskRecord};
pub use team::{TeamMember, TeamMemberRecord};
pub use time::Timestamp;
pub use workspace::{
    DEFAULT_WORKSPACE_NAME, Workspace, WorkspaceRecord, next_untitled_name, validate_name,
};
