-- Every review a task has had. The latest one is also kept as a setting for the panel, but this table is
-- the history: what the verdict was and when, and which model gave it. review_json is the whole review as the
-- reviewer's reply was read, so an old review can be shown the way it was.
CREATE TABLE task_reviews (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id     TEXT    NOT NULL REFERENCES tasks (id) ON DELETE CASCADE,
    outcome     TEXT    NOT NULL CHECK (length(trim(outcome)) > 0),
    summary     TEXT    NOT NULL,
    review_json TEXT    NOT NULL,
    model       TEXT,
    created_at  INTEGER NOT NULL
);

CREATE INDEX task_reviews_by_task ON task_reviews (task_id, created_at);
