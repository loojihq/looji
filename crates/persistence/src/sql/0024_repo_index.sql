-- The cached index of a project's repository: its file tree, each source file's hash, and the symbols read from it.
-- A file's symbols are owned by its row in index_files, so deleting the row removes them. A row whose parser_version
-- is older than the current one is parsed again, even when its hash still matches.
CREATE TABLE index_entries (
    project_id TEXT NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    path       TEXT NOT NULL,
    kind       TEXT NOT NULL CHECK (kind IN ('directory', 'file', 'symlink')),
    PRIMARY KEY (project_id, path)
);

CREATE TABLE index_files (
    project_id     TEXT NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    path           TEXT NOT NULL,
    hash           TEXT NOT NULL,
    parser_version INTEGER NOT NULL,
    skip_reason    TEXT,
    PRIMARY KEY (project_id, path)
);

CREATE TABLE index_symbols (
    project_id TEXT NOT NULL,
    path       TEXT NOT NULL,
    name       TEXT NOT NULL,
    kind       TEXT NOT NULL CHECK (kind IN ('function', 'type', 'module')),
    start_line INTEGER NOT NULL CHECK (start_line >= 1),
    end_line   INTEGER NOT NULL CHECK (end_line >= start_line),
    FOREIGN KEY (project_id, path) REFERENCES index_files (project_id, path) ON DELETE CASCADE
);

CREATE INDEX index_symbols_by_file ON index_symbols (project_id, path);
