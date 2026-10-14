-- The list view reads tasks one page at a time, in manual order. The index on (project_id, position)
-- from the first migration lets a page start at its cursor, but two tasks can share a position key,
-- so the page's sort also needs the id. This replaces that index with one that ends in the id, so a
-- page deep in the list costs about the same as the first, and the sort never needs a temporary b-tree.
DROP INDEX tasks_by_project_position;
CREATE INDEX tasks_by_project_position ON tasks (project_id, position, id);
