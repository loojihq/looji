-- Team members belong to one project, and each one gets a colour (an index into the palette) when it's
-- added. The workspace-scoped table from the first migration had no writer, so no member could ever
-- be placed in a project: its rows are dropped, not converted. Tasks that point at them are cleared
-- first, so none is left pointing at a missing row.
--
-- A task's assignee_id keeps its ON DELETE SET NULL, so removing a member unassigns its tasks and
-- never deletes them. Removal also clears the column explicitly, so it doesn't rely on foreign keys
-- being enabled.

UPDATE tasks SET assignee_id = NULL WHERE assignee_id IS NOT NULL;

DROP TABLE team_members;

CREATE TABLE team_members (
    id         TEXT    PRIMARY KEY NOT NULL,
    project_id TEXT    NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    name       TEXT    NOT NULL,
    colour     INTEGER NOT NULL CHECK (colour BETWEEN 0 AND 5),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX team_members_by_project ON team_members (project_id);

-- Finds a member's tasks without a full scan, for the filters and the workload view.
CREATE INDEX tasks_by_assignee ON tasks (assignee_id);
