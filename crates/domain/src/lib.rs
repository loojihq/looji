//! Plain Rust domain types and rules.
//!
//! This crate must not depend on GPUI or SQLite. Storage is described by the
//! repository traits in [`repository`]; the persistence crate implements them.

mod activity;
mod board;
mod code_source;
mod due_calculation;
mod error;
mod file_tree;
mod ids;
mod list_view;
mod ordering;
mod plan_draft;
mod plan_wizard;
mod position;
pub mod preferences;
mod project;
mod project_view;
pub mod repository;
mod schedule_draft;
mod status;
mod status_columns;
mod status_visibility;
mod task;
mod task_event;
mod task_fields;
mod task_filter;
mod task_move;
mod task_query;
mod team;
mod time;
mod validation;
mod working_schedule;
mod working_time;
mod workload;
mod workspace;
mod wsl_path;
mod zone;

pub use activity::{ActivityKind, TaskActivity};
pub use board::{Column, ColumnRecord};
pub use code_source::{CodeSourceError, CodeSourceStatus, PathCheck, clean_path_input, status_for};
pub use due_calculation::{
    LocalMoment, add_working_hours, add_working_minutes, needs_estimate_hint, schedules_on_move,
};
pub use error::{DomainError, RepoError, RepoResult};
pub use file_tree::{BUILT_IN_IGNORED, EntryKind, TreeNode, build_tree, is_built_in_ignored};
pub use ids::PlanDraftId;
pub use ids::{ActivityId, ColumnId, ProjectId, TaskId, TeamMemberId, WorkspaceId};
pub use list_view::{ListColumn, ListSort, ListView, SortColumn, SortDirection, ToggleOutcome};
pub use ordering::{
    FIRST_KEY, REBALANCE_LENGTH, after, before, key_between, needs_rebalance, rebalance,
};
pub use plan_draft::{
    DraftState, PlanDraft, PlanDraftError, PlanDraftParts, PlanEdit, PlanStory, PlanTask,
    Publication, PublishPlan, RefineTarget, RefinedPart,
};
pub use plan_wizard::{
    Generation, MAX_GOAL_CHARS, PlanWizard, WizardError, WizardPhase, validate_goal,
};
pub use position::Position;
pub use preferences::{
    DefaultView, Preference, Preferences, Theme as PreferredTheme, UpdateChannel,
};
pub use project::{
    MAX_DESCRIPTION_CHARS, Project, ProjectRecord, clean_description, validate_project_name,
};
pub use project_view::{ProjectView, ProjectViewState, ScrollOffset, scroll_key};
pub use repository::{
    CodeSourceRepository, ColumnRepository, ListViewRepository, PlanDraftRepository,
    ProjectFilterRepository, ProjectRepository, ProjectViewRepository, SettingsRepository,
    TaskRepository, TeamMemberRepository, WorkspaceRepository,
};
pub use schedule_draft::{
    DraftIssue, PREVIEW_COUNT, SAMPLE_ESTIMATE_HOURS, ScheduleDraft, TIME_FORMAT_MESSAGE,
    TimeField, parse_time, preview_due_dates,
};
pub use status::Status;
pub use status_columns::{StatusColumn, status_columns};
pub use status_visibility::{check_hideable, resolve_visible_status, visible_statuses};
pub use task::{Estimate, Task, TaskRecord};
pub use task_event::{TaskEvent, TaskEventKind, changed_fields};
pub use task_fields::{
    DueDate, EstimateError, MAX_TAG_CHARS, MAX_TAGS, Priority, TagError, normalize_tag,
    normalize_tags, parse_due_date, parse_estimate_hours, validate_title,
};
pub use task_filter::{TagMatch, TaskFilter};
pub use task_move::TaskMove;
pub use task_query::{
    PAGE_SIZE, PageCursor, SortValue, TaskPage, TaskQuery, contains_ignoring_case, lowercase,
    search_terms, term_matches,
};
pub use team::{
    MAX_MEMBER_NAME_CHARS, MEMBER_COLOUR_COUNT, MemberColour, TeamMember, TeamMemberRecord,
    display_labels, initials, validate_member_name,
};
pub use time::Timestamp;
pub use validation::MAX_TEXT_CHARS;
pub use working_schedule::{
    DEFAULT_END_MINUTES, DEFAULT_START_MINUTES, MINUTES_PER_DAY, ScheduleError, Workdays,
    WorkingSchedule, format_time,
};
pub use working_time::{
    Period, WorkingCalendar, available_minutes, format_hours, weekday, working_days,
};
pub use workload::{OVERLOAD_PERCENT, Owner, WorkloadReport, WorkloadRow, workload};
pub use workspace::{
    DEFAULT_WORKSPACE_NAME, Workspace, WorkspaceRecord, next_untitled_name, validate_name,
};
pub use wsl_path::{WslPath, parse_wsl_path};
pub use zone::{Zone, ZoneError, local_to_timestamp, timestamp_to_local, work_start};
