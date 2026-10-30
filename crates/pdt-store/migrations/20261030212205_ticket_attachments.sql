-- A ticket's attachments (PDT-300): screenshots and text files, stored in
-- `<data dir>/tickets/<ticket id>/attachments/<id>.<ext>`; this is their metadata. Removed with the ticket (pdtd
-- deletes the folder).
CREATE TABLE ticket_attachments (
    id         TEXT PRIMARY KEY NOT NULL,
    ticket_id  TEXT NOT NULL REFERENCES tickets(id) ON DELETE CASCADE,
    name       TEXT NOT NULL,
    media_type TEXT NOT NULL,
    size       INTEGER NOT NULL,
    actor      TEXT NOT NULL,              -- user | agent
    agent      TEXT,                       -- the agent session that added it (AgentBy JSON)
    event_id   TEXT,                       -- the comment it came with
    created_at INTEGER NOT NULL
);
CREATE INDEX ticket_attachments_by_ticket ON ticket_attachments (ticket_id, created_at);
