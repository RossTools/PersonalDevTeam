-- Ticket parts (PDT-38): a part names its parent (one level; the store refuses to delete a parent with parts. Not
-- RESTRICT: removing a project cascades to its tickets in any order).
ALTER TABLE tickets ADD COLUMN parent_id TEXT REFERENCES tickets(id) ON DELETE SET NULL;
CREATE INDEX tickets_by_parent ON tickets (parent_id, rank) WHERE parent_id IS NOT NULL;
