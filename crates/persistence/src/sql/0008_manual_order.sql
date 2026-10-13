-- Manual ordering. Each task's `position` is a fractional key that orders it within its status
-- column. Existing tasks are renumbered in their current display order, so the board looks the
-- same after the upgrade.

-- Fixed-width keys sort in their numeric order. The trailing '5' keeps the last character
-- off the smallest digit, as real keys require. The ranking reads the old keys before any
-- row changes, so the order comes from the data as it was.
UPDATE tasks SET position = ranked.key
FROM (
    SELECT id,
           'a' || printf('%09d', ROW_NUMBER() OVER (
               PARTITION BY project_id, status ORDER BY position COLLATE BINARY, id)) || '5'
           AS key
    FROM tasks
) AS ranked
WHERE ranked.id = tasks.id;

-- Created after the rewrite, so duplicate keys fail loudly here rather than during the update.
CREATE UNIQUE INDEX tasks_position_unique ON tasks (project_id, status, position);
