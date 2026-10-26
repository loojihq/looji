-- Files attached to tasks. The file itself is in the attachments folder under stored_name. A row goes away with
-- its task, and the file is removed by the app when the attachment is removed.
CREATE TABLE task_attachments (
    task_id     TEXT    NOT NULL REFERENCES tasks (id) ON DELETE CASCADE,
    stored_name TEXT    NOT NULL CHECK (length(trim(stored_name)) > 0),
    file_name   TEXT    NOT NULL CHECK (length(trim(file_name)) > 0),
    kind        TEXT    NOT NULL CHECK (kind IN ('image', 'video', 'file')),
    created_at  INTEGER NOT NULL,
    PRIMARY KEY (task_id, stored_name)
);
