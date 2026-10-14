-- The edit history of a task: when it was created, and which fields were edited. Status moves are
-- recorded in task_activity. Each row is written in the same transaction as the change it records.
-- `payload` is a JSON array of the field names that changed, for edits; it's empty otherwise.

CREATE TABLE task_events (
    id         TEXT    PRIMARY KEY NOT NULL,
    task_id    TEXT    NOT NULL REFERENCES tasks (id) ON DELETE CASCADE,
    kind       TEXT    NOT NULL CHECK (kind IN ('created', 'edited')),
    payload    TEXT    NOT NULL DEFAULT '[]',
    created_at INTEGER NOT NULL
);

CREATE INDEX task_events_by_task ON task_events (task_id, created_at);
