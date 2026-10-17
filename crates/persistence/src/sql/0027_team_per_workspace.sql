-- Team members belong to a workspace, not a project, so one team is shared by every project in it.
--
-- Each existing member moves to its project's workspace. A name that already appears in that workspace
-- (ignoring case) keeps the first member's row, and the others are dropped. Assignments to a dropped
-- member are moved to the kept one, so no task or draft task loses its assignee.
--
-- Foreign keys are on, and the assignee columns use ON DELETE SET NULL. Dropping the old table would
-- therefore clear every assignment, so the assignments are saved, cleared, and restored around the swap.

CREATE TEMP TABLE member_map AS
SELECT m.id AS old_id,
       (SELECT k.id
          FROM team_members k
          JOIN projects pk ON pk.id = k.project_id
         WHERE pk.workspace_id = p.workspace_id
           AND lower(k.name) = lower(m.name)
         ORDER BY k.rowid
         LIMIT 1) AS new_id
  FROM team_members m
  JOIN projects p ON p.id = m.project_id;

CREATE TEMP TABLE task_assignments AS
SELECT t.id AS task_id,
       (SELECT new_id FROM member_map WHERE old_id = t.assignee_id) AS member_id
  FROM tasks t
 WHERE t.assignee_id IS NOT NULL;

CREATE TEMP TABLE draft_assignments AS
SELECT d.draft_id AS draft_id,
       d.position AS position,
       (SELECT new_id FROM member_map WHERE old_id = d.assignee_id) AS member_id
  FROM plan_draft_tasks d
 WHERE d.assignee_id IS NOT NULL;

UPDATE tasks SET assignee_id = NULL WHERE assignee_id IS NOT NULL;
UPDATE plan_draft_tasks SET assignee_id = NULL WHERE assignee_id IS NOT NULL;

CREATE TABLE team_members_by_workspace (
    id           TEXT    PRIMARY KEY NOT NULL,
    workspace_id TEXT    NOT NULL REFERENCES workspaces (id) ON DELETE CASCADE,
    name         TEXT    NOT NULL,
    colour       INTEGER NOT NULL CHECK (colour BETWEEN 0 AND 5),
    created_at   INTEGER NOT NULL,
    updated_at   INTEGER NOT NULL
);

INSERT INTO team_members_by_workspace (id, workspace_id, name, colour, created_at, updated_at)
SELECT m.id, p.workspace_id, m.name, m.colour, m.created_at, m.updated_at
  FROM team_members m
  JOIN projects p ON p.id = m.project_id
 WHERE m.id IN (SELECT new_id FROM member_map);

DROP TABLE team_members;
ALTER TABLE team_members_by_workspace RENAME TO team_members;
CREATE INDEX team_members_by_workspace_index ON team_members (workspace_id);

UPDATE tasks
   SET assignee_id = (SELECT member_id FROM task_assignments WHERE task_id = tasks.id)
 WHERE id IN (SELECT task_id FROM task_assignments);

UPDATE plan_draft_tasks
   SET assignee_id = (SELECT member_id FROM draft_assignments
                       WHERE draft_id = plan_draft_tasks.draft_id
                         AND position = plan_draft_tasks.position)
 WHERE (draft_id, position) IN (SELECT draft_id, position FROM draft_assignments);

DROP TABLE draft_assignments;
DROP TABLE task_assignments;
DROP TABLE member_map;
