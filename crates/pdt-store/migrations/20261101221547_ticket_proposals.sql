-- Ticket proposals (PDT-366): a ticket an agent made is held for the user from the moment it is made until it leaves
-- Draft. Ticket lists leave held tickets out.
ALTER TABLE tickets ADD COLUMN proposed_at INTEGER;
