-- Secrets for remote sessions and updating instances together (PDT-637).
-- On the owner: the instance a forwarded use of one of its secrets went to (NULL: used here).
ALTER TABLE secret_uses ADD COLUMN instance TEXT;
-- On the owner: the names of the secrets a joined instance keeps for the project itself (JSON list, from its
-- heartbeat), and their uses there as it reported them. Never their values.
ALTER TABLE remote_instances ADD COLUMN secrets TEXT NOT NULL DEFAULT '[]';
CREATE TABLE remote_secret_uses (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    link_id TEXT NOT NULL REFERENCES remote_instances(link_id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    at INTEGER NOT NULL,
    by TEXT NOT NULL,
    how TEXT NOT NULL,
    session_id TEXT
);
CREATE INDEX remote_secret_uses_by_link ON remote_secret_uses (link_id, name, at);
-- On a remote instance: the project updates on its own channel rather than its owner's.
ALTER TABLE remote_projects ADD COLUMN own_update_channel INTEGER NOT NULL DEFAULT 0;
