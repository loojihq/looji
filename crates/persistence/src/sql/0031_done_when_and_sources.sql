-- A planned task can carry the points that say it is done, one per line. Publishing writes them into the task's
-- description under "Done when:". A reply from the AI can carry the sources it drew on, as JSON.
ALTER TABLE plan_draft_tasks ADD COLUMN done_when TEXT NOT NULL DEFAULT '';
ALTER TABLE task_messages ADD COLUMN sources TEXT NOT NULL DEFAULT '';
