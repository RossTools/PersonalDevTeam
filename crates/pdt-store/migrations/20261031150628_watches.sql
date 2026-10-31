-- Watches (PDT-319): a ticket or workflow run a PM conversation (session_id) or the user (NULL) hears about.
CREATE TABLE watches (
    id TEXT PRIMARY KEY,
    target TEXT NOT NULL,
    target_id TEXT NOT NULL,
    session_id TEXT,
    events TEXT NOT NULL,
    once INTEGER NOT NULL,
    cursor INTEGER NOT NULL,
    inbox_since INTEGER NOT NULL,
    run_status TEXT,
    pending TEXT NOT NULL DEFAULT '[]',
    created_at INTEGER NOT NULL,
    ended_at INTEGER
);
CREATE UNIQUE INDEX watches_one_per_watcher ON watches (target, target_id, COALESCE(session_id, ''))
    WHERE ended_at IS NULL;
CREATE INDEX watches_session ON watches (session_id) WHERE ended_at IS NULL;
