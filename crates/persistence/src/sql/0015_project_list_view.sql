-- The list view's settings for a project: the sort, and the columns shown in order. The columns are a
-- JSON array of column names, so their order can change without a schema change. A project with no row
-- uses the defaults. The row goes with its project, so deleting the project removes it.
CREATE TABLE project_list_view (
    project_id      TEXT PRIMARY KEY NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    sort_column     TEXT NOT NULL,
    sort_direction  TEXT NOT NULL CHECK (sort_direction IN ('asc', 'desc')),
    visible_columns TEXT NOT NULL
);
