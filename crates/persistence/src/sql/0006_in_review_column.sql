-- Adds the In review status and the per-project column toggle.
--
-- SQLite can't alter a CHECK constraint in place, so the two tables that store statuses are
-- rebuilt. Nothing references tasks or board_columns, so dropping them is safe.
--
-- tasks.hidden_from records the status a task had before its column was hidden, so showing the
-- column again can put it back. NULL means the task hasn't been moved by a hide.

CREATE TABLE board_columns_new (
    id         TEXT    PRIMARY KEY NOT NULL,
    project_id TEXT    NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    status     TEXT    NOT NULL CHECK (status IN ('backlog', 'todo', 'in_progress', 'in_review', 'done')),
    name       TEXT    NOT NULL,
    position   TEXT    NOT NULL,
    visible    INTEGER NOT NULL CHECK (visible IN (0, 1)),
    UNIQUE (project_id, status)
);

INSERT INTO board_columns_new (id, project_id, status, name, position, visible)
    SELECT id, project_id, status, name, position, visible FROM board_columns;

DROP TABLE board_columns;

ALTER TABLE board_columns_new RENAME TO board_columns;

CREATE TABLE tasks_new (
    id               TEXT    PRIMARY KEY NOT NULL,
    project_id       TEXT    NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    title            TEXT    NOT NULL,
    description      TEXT    NOT NULL DEFAULT '',
    status           TEXT    NOT NULL CHECK (status IN ('backlog', 'todo', 'in_progress', 'in_review', 'done')),
    position         TEXT    NOT NULL,
    estimate_minutes INTEGER CHECK (estimate_minutes IS NULL OR estimate_minutes >= 0),
    assignee_id      TEXT    REFERENCES team_members (id) ON DELETE SET NULL,
    hidden_from      TEXT    CHECK (hidden_from IS NULL OR hidden_from = 'in_review'),
    created_at       INTEGER NOT NULL,
    updated_at       INTEGER NOT NULL
);

INSERT INTO tasks_new (id, project_id, title, description, status, position, estimate_minutes,
                       assignee_id, created_at, updated_at)
    SELECT id, project_id, title, description, status, position, estimate_minutes,
           assignee_id, created_at, updated_at FROM tasks;

DROP TABLE tasks;

ALTER TABLE tasks_new RENAME TO tasks;

CREATE INDEX tasks_by_project_position ON tasks (project_id, position);

ALTER TABLE projects ADD COLUMN show_in_review INTEGER NOT NULL DEFAULT 1 CHECK (show_in_review IN (0, 1));
