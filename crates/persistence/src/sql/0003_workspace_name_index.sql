-- Workspace names are unique regardless of case. The domain checks this first;
-- this index stops a race or a bug from writing a duplicate.
-- SQLite's lower() only folds ASCII letters, so non-ASCII names are checked by the domain alone.

CREATE UNIQUE INDEX workspaces_name_unique ON workspaces (lower(name));
