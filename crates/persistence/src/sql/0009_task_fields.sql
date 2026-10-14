-- Fields for the create and edit dialog: priority, due date, and tags. A due date is a plain
-- calendar day in the form YYYY-MM-DD. Tags live in their own table, so filtering by tag can use
-- an index later.

ALTER TABLE tasks ADD COLUMN priority TEXT
    CHECK (priority IS NULL OR priority IN ('low', 'medium', 'high'));

ALTER TABLE tasks ADD COLUMN due_on TEXT
    CHECK (due_on IS NULL OR due_on GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]');

CREATE TABLE task_tags (
    task_id TEXT NOT NULL REFERENCES tasks (id) ON DELETE CASCADE,
    tag     TEXT NOT NULL CHECK (length(tag) > 0),
    PRIMARY KEY (task_id, tag)
);

CREATE INDEX task_tags_by_tag ON task_tags (tag);
