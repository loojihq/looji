use std::collections::BTreeMap;

use looji_domain::{
    ProjectId, ProjectView, ProjectViewRepository, ProjectViewState, RepoResult, ScrollOffset,
    Timestamp,
};
use rusqlite::params;
use serde_json::{Map, Value};

use super::{execute, get, id_text, query_one};
use crate::SqliteStore;

/// Reads the stored scroll JSON. A value that doesn't parse, or has entries in the wrong shape, keeps only
/// the entries that do, so a damaged row costs scroll positions, not the view.
fn read_scroll(json: &str) -> BTreeMap<String, ScrollOffset> {
    let Ok(Value::Object(entries)) = serde_json::from_str::<Value>(json) else {
        return BTreeMap::new();
    };
    entries
        .into_iter()
        .filter_map(|(key, value)| {
            let x = value.get("x")?.as_f64()? as f32;
            let y = value.get("y")?.as_f64()? as f32;
            (x.is_finite() && y.is_finite()).then_some((key, ScrollOffset { x, y }))
        })
        .collect()
}

fn write_scroll(scroll: &BTreeMap<String, ScrollOffset>) -> String {
    let mut entries = Map::new();
    for (key, offset) in scroll {
        let mut place = Map::new();
        place.insert("x".into(), Value::from(f64::from(offset.x)));
        place.insert("y".into(), Value::from(f64::from(offset.y)));
        entries.insert(key.clone(), Value::Object(place));
    }
    Value::Object(entries).to_string()
}

impl ProjectViewRepository for SqliteStore {
    /// A project with no row reads as the default, and no row is created to read it.
    fn load_view_state(&self, project: ProjectId) -> RepoResult<ProjectViewState> {
        let stored = query_one(
            &self.connection,
            "SELECT active_view, scroll_json FROM project_view_state WHERE project_id = ?1",
            params![id_text(project.as_uuid())],
            |row| Ok((get::<String>(row, 0)?, get::<String>(row, 1)?)),
        )?;
        Ok(match stored {
            None => ProjectViewState::default(),
            Some((view, scroll)) => ProjectViewState {
                view: ProjectView::parse(&view).unwrap_or_default(),
                scroll: read_scroll(&scroll),
            },
        })
    }

    fn save_view_state(&self, project: ProjectId, state: &ProjectViewState) -> RepoResult<()> {
        execute(
            &self.connection,
            "INSERT INTO project_view_state (project_id, active_view, scroll_json, updated_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (project_id) DO UPDATE SET
                 active_view = excluded.active_view,
                 scroll_json = excluded.scroll_json,
                 updated_at = excluded.updated_at",
            params![
                id_text(project.as_uuid()),
                state.view.name(),
                write_scroll(&state.scroll),
                Timestamp::now().unix_millis(),
            ],
        )
    }
}
