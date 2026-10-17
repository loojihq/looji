-- Lets the Backlog column be hidden, as In review can be. Hiding a column moves its tasks to the nearest
-- shown column and records where they came from in tasks.hidden_from, so showing the column again can put
-- them back.
--
-- hidden_from's CHECK only allows 'in_review', so tasks is rebuilt to allow 'backlog' too. Activity, events,
-- chat, notes and tags all reference tasks with ON DELETE CASCADE. Dropping tasks with foreign keys on would
-- delete those rows, so the migration runner turns foreign keys off while this runs and checks them before
-- the migration commits.

CREATE TABLE tasks_new (
    id               TEXT    PRIMARY KEY NOT NULL,
    project_id       TEXT    NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    title            TEXT    NOT NULL,
    description      TEXT    NOT NULL DEFAULT '',
    status           TEXT    NOT NULL CHECK (status IN ('backlog', 'todo', 'in_progress', 'in_review', 'done')),
    position         TEXT    NOT NULL,
    estimate_minutes INTEGER CHECK (estimate_minutes IS NULL OR estimate_minutes >= 0),
    assignee_id      TEXT    REFERENCES team_members (id) ON DELETE SET NULL,
    hidden_from      TEXT    CHECK (hidden_from IS NULL OR hidden_from IN ('backlog', 'in_review')),
    created_at       INTEGER NOT NULL,
    updated_at       INTEGER NOT NULL,
    priority         TEXT    CHECK (priority IS NULL OR priority IN ('low', 'medium', 'high')),
    due_on           TEXT    CHECK (due_on IS NULL OR due_on GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]'),
    start_on         TEXT    CHECK (start_on IS NULL OR start_on GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]')
);

INSERT INTO tasks_new (id, project_id, title, description, status, position, estimate_minutes,
                       assignee_id, hidden_from, created_at, updated_at, priority, due_on, start_on)
    SELECT id, project_id, title, description, status, position, estimate_minutes,
           assignee_id, hidden_from, created_at, updated_at, priority, due_on, start_on FROM tasks;

DROP TABLE tasks;

ALTER TABLE tasks_new RENAME TO tasks;

CREATE INDEX tasks_by_project_position ON tasks (project_id, position, id);
CREATE INDEX tasks_by_assignee ON tasks (assignee_id);
CREATE UNIQUE INDEX tasks_position_unique ON tasks (project_id, status, position);

ALTER TABLE projects ADD COLUMN show_backlog INTEGER NOT NULL DEFAULT 1 CHECK (show_backlog IN (0, 1));
