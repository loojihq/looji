-- When a plan was published to the backlog. A published plan can't change, and publishing it again is refused.
ALTER TABLE plan_drafts ADD COLUMN published_at INTEGER;
