-- Project descriptions, and names unique within a workspace ignoring case.
-- The domain checks the same rule first; this index is the backstop.
-- SQLite lower() folds only ASCII letters, so non-ASCII names rely on the domain check.

ALTER TABLE projects ADD COLUMN description TEXT;

CREATE UNIQUE INDEX projects_name_unique ON projects (workspace_id, lower(name));
