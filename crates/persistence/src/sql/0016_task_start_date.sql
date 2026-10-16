-- When a task's work starts, as a calendar day in the form YYYY-MM-DD. Like the due date, it has no time and
-- no zone. Both dates are optional. A task with both can be drawn on the roadmap. A start after the due date
-- is stored as given, so the roadmap can show it.
ALTER TABLE tasks ADD COLUMN start_on TEXT
    CHECK (start_on IS NULL OR start_on GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]');
