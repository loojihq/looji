-- The board filter each project last used. There's one row per selected value, under a facet name
-- (status, priority, assignee, or tag). A tag mode of "all" is one row under tag_mode, and "any" is
-- the default, so it isn't stored. The rows belong to their project, so deleting a project removes
-- them, and so does deleting its workspace.

CREATE TABLE project_filters (
    project_id TEXT NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    facet      TEXT NOT NULL CHECK (facet IN ('status', 'priority', 'assignee', 'tag', 'tag_mode')),
    value      TEXT NOT NULL,
    PRIMARY KEY (project_id, facet, value)
);
