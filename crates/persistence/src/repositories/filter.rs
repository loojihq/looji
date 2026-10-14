use looji_domain::{
    Priority, ProjectFilterRepository, ProjectId, RepoResult, Status, TagMatch, TaskFilter,
    TeamMemberId,
};
use rusqlite::params;
use uuid::Uuid;

use super::{execute, get, id_text, query_all, storage};
use crate::SqliteStore;

/// Stored as "none" for the choice of no priority, or for unassigned tasks.
const NONE: &str = "none";

/// Reads one stored row into the filter. A row whose value no longer parses is left out, so a
/// damaged or outdated row can't break the board.
fn read_row(filter: &mut TaskFilter, facet: &str, value: &str) {
    match facet {
        "status" => {
            if let Ok(status) = value.parse::<Status>() {
                filter.statuses.push(status);
            }
        }
        "priority" if value == NONE => filter.priorities.push(None),
        "priority" => {
            if let Ok(priority) = value.parse::<Priority>() {
                filter.priorities.push(Some(priority));
            }
        }
        "assignee" if value == NONE => filter.assignees.push(None),
        "assignee" => {
            if let Ok(id) = Uuid::parse_str(value) {
                filter.assignees.push(Some(TeamMemberId::from_uuid(id)));
            }
        }
        "tag" => filter.tags.push(value.to_string()),
        "tag_mode" if value == "all" => filter.tag_match = TagMatch::All,
        _ => {}
    }
}

impl ProjectFilterRepository for SqliteStore {
    fn load_filter(&self, project: ProjectId) -> RepoResult<TaskFilter> {
        let rows = query_all(
            &self.connection,
            "SELECT facet, value FROM project_filters WHERE project_id = ?1 ORDER BY facet, value",
            params![id_text(project.as_uuid())],
            |row| Ok((get::<String>(row, 0)?, get::<String>(row, 1)?)),
        )?;
        let mut filter = TaskFilter::default();
        for (facet, value) in rows {
            read_row(&mut filter, &facet, &value);
        }
        Ok(filter)
    }

    /// Replaces the project's rows in one transaction, so a save is all or nothing.
    fn save_filter(&self, project: ProjectId, filter: &TaskFilter) -> RepoResult<()> {
        let key = id_text(project.as_uuid());
        let transaction = self.connection.unchecked_transaction().map_err(storage)?;
        execute(
            &transaction,
            "DELETE FROM project_filters WHERE project_id = ?1",
            params![key],
        )?;
        let mut rows: Vec<(&str, String)> = Vec::new();
        rows.extend(
            filter
                .statuses
                .iter()
                .map(|status| ("status", status.as_str().to_string())),
        );
        rows.extend(filter.priorities.iter().map(|priority| {
            (
                "priority",
                priority.map_or(NONE.to_string(), |p| p.as_str().to_string()),
            )
        }));
        rows.extend(filter.assignees.iter().map(|assignee| {
            (
                "assignee",
                assignee.map_or(NONE.to_string(), |id| id_text(id.as_uuid())),
            )
        }));
        rows.extend(filter.tags.iter().map(|tag| ("tag", tag.clone())));
        if filter.tag_match == TagMatch::All {
            rows.push(("tag_mode", "all".to_string()));
        }
        for (facet, value) in rows {
            execute(
                &transaction,
                "INSERT INTO project_filters (project_id, facet, value) VALUES (?1, ?2, ?3)",
                params![key, facet, value],
            )?;
        }
        transaction.commit().map_err(storage)
    }
}
