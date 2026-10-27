-- Images, videos and files kept with a section of a project's blueprint, such as a sketch or a screenshot. They belong
-- to the section (its kind), not to one version of it, so a new version keeps them. The file itself is in the
-- attachments folder under stored_name.
CREATE TABLE blueprint_media (
    project_id  TEXT    NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    doc_kind    TEXT    NOT NULL,
    stored_name TEXT    NOT NULL CHECK (length(trim(stored_name)) > 0),
    file_name   TEXT    NOT NULL CHECK (length(trim(file_name)) > 0),
    kind        TEXT    NOT NULL CHECK (kind IN ('image', 'video', 'file')),
    created_at  INTEGER NOT NULL,
    PRIMARY KEY (project_id, doc_kind, stored_name)
);
