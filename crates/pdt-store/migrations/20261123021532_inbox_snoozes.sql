-- Snoozed Inbox items: hidden until `until_ms`, or until ticket `ticket_id` is closed, whichever comes first. Keyed by
-- the item's id, so items the Inbox lists without storing them (approvals, sessions waiting) can be snoozed too.
CREATE TABLE inbox_snoozes (
    item_id TEXT PRIMARY KEY,
    until_ms INTEGER,
    ticket_id TEXT,
    created_at INTEGER NOT NULL
);
