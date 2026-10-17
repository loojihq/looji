-- The local repository folder a project is connected to. A project with no row is not connected. The row goes
-- with its project, so deleting a project, or the workspace that holds it, removes it. The path is the one the
-- user chose, kept as typed (trimmed); a folder that later disappears keeps its row, and only Disconnect removes it.
CREATE TABLE project_repository (
    project_id   TEXT PRIMARY KEY NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    path         TEXT NOT NULL CHECK (length(trim(path)) > 0),
    connected_at INTEGER NOT NULL
);
