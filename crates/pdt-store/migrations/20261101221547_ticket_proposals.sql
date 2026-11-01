-- Ticket proposals (PDT-366): a ticket an agent made is held for the user from the moment it is made until it leaves
-- Draft (agents can't move it on). Ticket lists leave out tickets with an open proposal item, and the board asks often.
ALTER TABLE tickets ADD COLUMN proposed_at INTEGER;
CREATE INDEX inbox_items_ticket_proposal ON inbox_items (ticket_id) WHERE kind = 'ticket_proposal' AND status = 'open';
