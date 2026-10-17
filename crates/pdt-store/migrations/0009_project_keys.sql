-- Project keys (workspaces spec §6.1). pdtd fills in keys for existing projects when the store opens.
ALTER TABLE projects ADD COLUMN key TEXT;
CREATE UNIQUE INDEX projects_key ON projects (key);
