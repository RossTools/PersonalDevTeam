-- Secrets agents can use but never see (PDT-158). Values live in the OS keychain only (account `secret:<id>`).
CREATE TABLE secrets (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL UNIQUE COLLATE NOCASE,
    description TEXT NOT NULL DEFAULT '',
    -- JSON lists of project and persona ids; empty: any.
    projects TEXT NOT NULL DEFAULT '[]',
    personas TEXT NOT NULL DEFAULT '[]',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    last_used_at INTEGER,
    last_used_by TEXT
);

-- Every use of a secret (the audit list).
CREATE TABLE secret_uses (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    secret_id TEXT NOT NULL REFERENCES secrets(id) ON DELETE CASCADE,
    at INTEGER NOT NULL,
    by TEXT NOT NULL,
    how TEXT NOT NULL,
    session_id TEXT,
    run_id TEXT
);
CREATE INDEX secret_uses_by_secret ON secret_uses (secret_id, at);
