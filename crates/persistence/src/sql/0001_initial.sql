-- Initial schema. Timestamps are UTC milliseconds since the Unix epoch.
-- IDs are hyphenated UUID text. Status values match domain::Status::as_str.

CREATE TABLE workspaces (
    id         TEXT    PRIMARY KEY NOT NULL,
    name       TEXT    NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE projects (
    id           TEXT    PRIMARY KEY NOT NULL,
    workspace_id TEXT    NOT NULL REFERENCES workspaces (id) ON DELETE CASCADE,
    name         TEXT    NOT NULL,
    created_at   INTEGER NOT NULL,
    updated_at   INTEGER NOT NULL
);

CREATE INDEX projects_by_workspace ON projects (workspace_id);

CREATE TABLE team_members (
    id           TEXT    PRIMARY KEY NOT NULL,
    workspace_id TEXT    NOT NULL REFERENCES workspaces (id) ON DELETE CASCADE,
    name         TEXT    NOT NULL,
    created_at   INTEGER NOT NULL,
    updated_at   INTEGER NOT NULL
);

CREATE INDEX team_members_by_workspace ON team_members (workspace_id);

CREATE TABLE board_columns (
    id         TEXT    PRIMARY KEY NOT NULL,
    project_id TEXT    NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    status     TEXT    NOT NULL CHECK (status IN ('backlog', 'todo', 'in_progress', 'done')),
    name       TEXT    NOT NULL,
    position   TEXT    NOT NULL,
    visible    INTEGER NOT NULL CHECK (visible IN (0, 1)),
    UNIQUE (project_id, status)
);

CREATE TABLE tasks (
    id               TEXT    PRIMARY KEY NOT NULL,
    project_id       TEXT    NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    title            TEXT    NOT NULL,
    description      TEXT    NOT NULL DEFAULT '',
    status           TEXT    NOT NULL CHECK (status IN ('backlog', 'todo', 'in_progress', 'done')),
    position         TEXT    NOT NULL,
    estimate_minutes INTEGER CHECK (estimate_minutes IS NULL OR estimate_minutes >= 0),
    assignee_id      TEXT    REFERENCES team_members (id) ON DELETE SET NULL,
    created_at       INTEGER NOT NULL,
    updated_at       INTEGER NOT NULL
);

CREATE INDEX tasks_by_project_position ON tasks (project_id, position);
