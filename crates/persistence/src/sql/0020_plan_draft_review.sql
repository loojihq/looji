-- The reviewed plan: the spec on the draft, and the priority, estimate, assignee, and tags of each task. The
-- assignee is cleared, not deleted, when its team member is removed, so a task is never left pointing at no one.
ALTER TABLE plan_drafts ADD COLUMN spec TEXT NOT NULL DEFAULT '';
ALTER TABLE plan_draft_tasks ADD COLUMN priority TEXT NOT NULL DEFAULT 'medium'
    CHECK (priority IN ('low', 'medium', 'high'));
ALTER TABLE plan_draft_tasks ADD COLUMN estimate_minutes INTEGER
    CHECK (estimate_minutes IS NULL OR estimate_minutes >= 0);
ALTER TABLE plan_draft_tasks ADD COLUMN assignee_id TEXT REFERENCES team_members (id) ON DELETE SET NULL;
ALTER TABLE plan_draft_tasks ADD COLUMN tags TEXT NOT NULL DEFAULT '';
