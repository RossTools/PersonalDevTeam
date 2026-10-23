-- Notifications (PDT-108): made once by pdtd, shown by every client. Only Show `inbox` and up are stored.
CREATE TABLE notifications (
    id TEXT PRIMARY KEY,
    -- Grows with every change to a row (created, updated, cleared): the stream's cursor.
    seq INTEGER NOT NULL,
    kind TEXT NOT NULL,
    needs_you INTEGER NOT NULL,
    severity TEXT NOT NULL,
    show TEXT NOT NULL,
    project_id TEXT,
    ticket_id TEXT,
    session_id TEXT,
    run_id TEXT,
    inbox_item_id TEXT,
    title TEXT NOT NULL,
    body TEXT NOT NULL,
    target TEXT NOT NULL,
    dedupe_key TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    read_at INTEGER,
    cleared_at INTEGER
);
CREATE UNIQUE INDEX notifications_seq ON notifications (seq);
-- One live row per cause.
CREATE UNIQUE INDEX notifications_live ON notifications (dedupe_key) WHERE cleared_at IS NULL;
CREATE INDEX notifications_feed ON notifications (needs_you, created_at);
