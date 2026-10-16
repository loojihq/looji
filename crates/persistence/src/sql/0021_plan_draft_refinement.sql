-- Which parts of a plan a person wrote or changed by hand. Refinement asks before it replaces one of these.
ALTER TABLE plan_drafts ADD COLUMN spec_edited INTEGER NOT NULL DEFAULT 0 CHECK (spec_edited IN (0, 1));
ALTER TABLE plan_draft_sections ADD COLUMN edited INTEGER NOT NULL DEFAULT 0 CHECK (edited IN (0, 1));
ALTER TABLE plan_draft_tasks ADD COLUMN edited INTEGER NOT NULL DEFAULT 0 CHECK (edited IN (0, 1));
