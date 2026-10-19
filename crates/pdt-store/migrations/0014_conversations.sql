-- Conversations (structured agents spec §4.2): PDT's own log of each agent conversation, as normalized events.
-- An item that changes (a tool call finishing, a request answered) keeps its row and pos but gets a new seq, so
-- "events after seq N" always includes every change; pos keeps conversation order.
CREATE TABLE session_events (
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    seq        INTEGER NOT NULL,
    pos        INTEGER NOT NULL,
    ts         INTEGER NOT NULL,
    kind       TEXT NOT NULL,
    item_id    TEXT,
    turn_id    TEXT,
    body       TEXT NOT NULL,
    raw        TEXT,
    PRIMARY KEY (session_id, seq)
);
CREATE INDEX session_events_by_item ON session_events (session_id, item_id);
-- terminal | conversation. Old rows are terminals.
ALTER TABLE sessions ADD COLUMN mode TEXT NOT NULL DEFAULT 'terminal';
ALTER TABLE sessions ADD COLUMN model TEXT;
-- ask | accept_edits | plan | bypass (conversations); backfilled from bypass_permissions when the store opens.
ALTER TABLE sessions ADD COLUMN permission_mode TEXT;
-- The agent's last transcript entry imported into the log (terminal handoff, 5c).
ALTER TABLE sessions ADD COLUMN history_from TEXT;
