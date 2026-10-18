-- The workspace Start made for the attempt (Start with "new workspace"), so finishing the ticket can remove it.
ALTER TABLE attempts ADD COLUMN workspace_id TEXT REFERENCES workspaces(id) ON DELETE SET NULL;
