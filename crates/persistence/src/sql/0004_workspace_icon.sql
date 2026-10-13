-- Workspace icon: the file name inside the icons directory, never an absolute path.

ALTER TABLE workspaces ADD COLUMN icon_file TEXT;
