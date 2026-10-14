-- Tags are stored in one form: trimmed, runs of spaces collapsed to one, and lowercase. The rows
-- that already exist are rewritten the same way, and rows that become duplicates are merged. The
-- CHECKs keep a non-normalised tag out of the table even if a write skips the domain rules.
--
-- SQLite's lower() only changes ASCII letters. Accented capitals in old rows are left alone here.
-- New tags are normalised in Rust, which handles all of Unicode. Rows that can't be normalised (a
-- tag that is only spaces, or one with a comma) are dropped. The app never stored those.

CREATE TABLE task_tags_normalised (
    task_id TEXT NOT NULL REFERENCES tasks (id) ON DELETE CASCADE,
    tag     TEXT NOT NULL CHECK (
        length(tag) > 0
        AND tag = trim(tag)
        AND tag NOT GLOB '*[A-Z]*'
        AND instr(tag, ',') = 0
    ),
    PRIMARY KEY (task_id, tag)
);

-- The replace chain turns each run of up to 32 spaces into one space. A tag is at most 32 characters.
INSERT OR IGNORE INTO task_tags_normalised (task_id, tag)
SELECT task_id, normalised FROM (
    SELECT task_id,
           replace(replace(replace(replace(replace(
               lower(trim(tag)),
               '  ', ' '), '  ', ' '), '  ', ' '), '  ', ' '), '  ', ' ') AS normalised
    FROM task_tags
)
WHERE normalised <> '' AND instr(normalised, ',') = 0;

DROP TABLE task_tags;

ALTER TABLE task_tags_normalised RENAME TO task_tags;

CREATE INDEX task_tags_by_tag ON task_tags (tag);
