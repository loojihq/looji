-- Planning drafts. A draft keeps its prompt, its state, the text received so far, and the sections and tasks
-- once the response was read. A draft is removed with its project.
CREATE TABLE plan_drafts (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    prompt TEXT NOT NULL CHECK (length(trim(prompt)) > 0),
    state TEXT NOT NULL CHECK (state IN ('generating', 'interrupted', 'ready', 'failed')),
    response TEXT NOT NULL DEFAULT '',
    recovered TEXT,
    failure TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX plan_drafts_by_project ON plan_drafts (project_id, created_at);

CREATE TABLE plan_draft_sections (
    draft_id TEXT NOT NULL REFERENCES plan_drafts (id) ON DELETE CASCADE,
    position INTEGER NOT NULL CHECK (position >= 0),
    title TEXT NOT NULL CHECK (length(trim(title)) > 0),
    body TEXT NOT NULL,
    PRIMARY KEY (draft_id, position)
);

CREATE TABLE plan_draft_tasks (
    draft_id TEXT NOT NULL REFERENCES plan_drafts (id) ON DELETE CASCADE,
    position INTEGER NOT NULL CHECK (position >= 0),
    title TEXT NOT NULL CHECK (length(trim(title)) > 0),
    section_position INTEGER CHECK (section_position >= 0),
    PRIMARY KEY (draft_id, position),
    FOREIGN KEY (draft_id, section_position) REFERENCES plan_draft_sections (draft_id, position) ON DELETE CASCADE
);
