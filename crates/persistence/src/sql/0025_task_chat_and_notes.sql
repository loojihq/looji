-- Conversations about a task, and the review notes made for it. Both go with their task.
CREATE TABLE task_threads (
    id         TEXT PRIMARY KEY NOT NULL,
    task_id    TEXT NOT NULL REFERENCES tasks (id) ON DELETE CASCADE,
    created_at INTEGER NOT NULL
);

CREATE INDEX task_threads_by_task ON task_threads (task_id);

CREATE TABLE task_messages (
    id         TEXT PRIMARY KEY NOT NULL,
    thread_id  TEXT NOT NULL REFERENCES task_threads (id) ON DELETE CASCADE,
    position   INTEGER NOT NULL CHECK (position >= 0),
    role       TEXT NOT NULL CHECK (role IN ('user', 'assistant')),
    content    TEXT NOT NULL,
    model      TEXT,
    created_at INTEGER NOT NULL,
    UNIQUE (thread_id, position)
);

CREATE TABLE task_review_notes (
    task_id  TEXT NOT NULL REFERENCES tasks (id) ON DELETE CASCADE,
    position INTEGER NOT NULL CHECK (position >= 0),
    kind     TEXT NOT NULL CHECK (kind IN ('risk', 'missing_test', 'check')),
    body     TEXT NOT NULL,
    files    TEXT NOT NULL DEFAULT '[]',
    model    TEXT,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (task_id, position)
);
