-- The view a project was last showing, and the scroll position of each scrollable place in it, as JSON. A
-- project with no row opens on the board. The row goes with its project, so deleting a project, or the
-- workspace that holds it, removes it.
CREATE TABLE project_view_state (
    project_id  TEXT PRIMARY KEY NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    active_view TEXT NOT NULL CHECK (active_view IN ('board', 'list', 'roadmap', 'workload')),
    scroll_json TEXT NOT NULL DEFAULT '{}',
    updated_at  INTEGER NOT NULL
);
