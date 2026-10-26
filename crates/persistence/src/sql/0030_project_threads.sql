-- Conversations about a whole project share task_threads, with the project's own id in task_id. A thread is no
-- longer tied to a task row, so the table is rebuilt without that foreign key, and two triggers take its threads
-- (and, through their cascade, their messages) away with the task or the project they belong to.
--
-- The migration runner turns foreign keys off while this runs, since task_messages refers to task_threads.

CREATE TABLE task_threads_new (
    id         TEXT PRIMARY KEY NOT NULL,
    task_id    TEXT NOT NULL,
    created_at INTEGER NOT NULL
);

INSERT INTO task_threads_new (id, task_id, created_at) SELECT id, task_id, created_at FROM task_threads;

DROP TABLE task_threads;
ALTER TABLE task_threads_new RENAME TO task_threads;

CREATE INDEX task_threads_by_task ON task_threads (task_id);

CREATE TRIGGER task_threads_with_task AFTER DELETE ON tasks
BEGIN
    DELETE FROM task_threads WHERE task_id = OLD.id;
END;

CREATE TRIGGER task_threads_with_project AFTER DELETE ON projects
BEGIN
    DELETE FROM task_threads WHERE task_id = OLD.id;
END;
