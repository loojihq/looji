//! Plain Rust domain types and rules.
//!
//! This crate must not depend on GPUI or SQLite. Storage is described by the
//! repository traits in [`repository`]; the persistence crate implements them.

mod activity;
mod board;
mod error;
mod ids;
mod ordering;
mod position;
pub mod preferences;
mod project;
pub mod repository;
mod status;
mod status_columns;
mod status_visibility;
mod task;
mod task_move;
mod team;
mod time;
mod validation;
mod workspace;

pub use activity::{ActivityKind, TaskActivity};
pub use board::{Column, ColumnRecord};
pub use error::{DomainError, RepoError, RepoResult};
pub use ids::{ActivityId, ColumnId, ProjectId, TaskId, TeamMemberId, WorkspaceId};
pub use ordering::{
    FIRST_KEY, REBALANCE_LENGTH, after, before, key_between, needs_rebalance, rebalance,
};
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
pub use status_visibility::{check_hideable, resolve_visible_status, visible_statuses};
pub use task::{Estimate, Task, TaskRecord};
pub use task_move::TaskMove;
pub use team::{TeamMember, TeamMemberRecord};
pub use time::Timestamp;
pub use workspace::{
    DEFAULT_WORKSPACE_NAME, Workspace, WorkspaceRecord, next_untitled_name, validate_name,
};
