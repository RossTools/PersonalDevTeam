-- Ticket parts (PDT-38): a part names its parent (one level; the store refuses to delete a parent with parts. Not
-- RESTRICT: removing a project cascades to its tickets in any order), and a parent
-- keeps the workspace its parts share.
ALTER TABLE tickets ADD COLUMN parent_id TEXT REFERENCES tickets(id) ON DELETE SET NULL;
ALTER TABLE tickets ADD COLUMN workspace_id TEXT REFERENCES workspaces(id) ON DELETE SET NULL;
CREATE INDEX tickets_by_parent ON tickets (parent_id, rank) WHERE parent_id IS NOT NULL;
