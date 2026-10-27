-- A project's blueprint: documents (an overview, the architecture, decisions, and so on) with the diagrams in each, as
-- JSON the diagram crate reads. Writing a new blueprint keeps the old one in blueprint_history, so a version can be
-- looked at later.
CREATE TABLE blueprint_docs (
    id         TEXT PRIMARY KEY NOT NULL,
    project_id TEXT NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    position   INTEGER NOT NULL,
    kind       TEXT NOT NULL,
    title      TEXT NOT NULL,
    body       TEXT NOT NULL,
    diagrams   TEXT NOT NULL DEFAULT '[]',
    version    INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE INDEX blueprint_docs_by_project ON blueprint_docs (project_id, position);

CREATE TABLE blueprint_history (
    id          INTEGER PRIMARY KEY,
    project_id  TEXT NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    version     INTEGER NOT NULL,
    position    INTEGER NOT NULL,
    kind        TEXT NOT NULL,
    title       TEXT NOT NULL,
    body        TEXT NOT NULL,
    diagrams    TEXT NOT NULL,
    updated_at  INTEGER NOT NULL
);
CREATE INDEX blueprint_history_by_project ON blueprint_history (project_id, version);
