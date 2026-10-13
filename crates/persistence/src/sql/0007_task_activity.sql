-- The activity history. Each row is one change to a task, written in the same transaction as
-- the change, so the history and the board can't disagree.

CREATE TABLE task_activity (
    id          TEXT    PRIMARY KEY NOT NULL,
    task_id     TEXT    NOT NULL REFERENCES tasks (id) ON DELETE CASCADE,
    kind        TEXT    NOT NULL CHECK (kind IN ('status_changed')),
    from_status TEXT    NOT NULL CHECK (from_status IN ('backlog', 'todo', 'in_progress', 'in_review', 'done')),
    to_status   TEXT    NOT NULL CHECK (to_status IN ('backlog', 'todo', 'in_progress', 'in_review', 'done')),
    created_at  INTEGER NOT NULL
);

CREATE INDEX task_activity_by_task ON task_activity (task_id, created_at);
