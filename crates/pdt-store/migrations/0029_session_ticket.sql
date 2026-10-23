-- PDT-105: a session that works on a ticket without being its attempt's (a workflow's Reviewer, QA or Refiner)
-- still belongs to the ticket. A session with an attempt takes its ticket from the attempt.
ALTER TABLE sessions ADD COLUMN ticket_id TEXT REFERENCES tickets(id) ON DELETE SET NULL;
CREATE INDEX sessions_ticket ON sessions (ticket_id) WHERE ticket_id IS NOT NULL;
