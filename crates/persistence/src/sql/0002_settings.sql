-- App-wide settings as key/value text. Holds the active workspace id.

CREATE TABLE settings (
    key   TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL
);
