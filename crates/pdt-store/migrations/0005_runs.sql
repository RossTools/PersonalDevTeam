-- Run instances (remote-host spec §4.2). A run's row outlives its process; active states have (or will have) one.
CREATE TABLE runs (
    id          TEXT PRIMARY KEY NOT NULL,
    project_id  TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    worktree_id TEXT REFERENCES worktrees(id) ON DELETE SET NULL,
    branch      TEXT NOT NULL,
    name        TEXT NOT NULL,
    kind        TEXT NOT NULL,
    state       TEXT NOT NULL,
    port        INTEGER,
    url         TEXT,
    data_dir    TEXT,
    cwd         TEXT NOT NULL,
    exit_code   INTEGER,
    error       TEXT,
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
);
CREATE INDEX runs_by_project ON runs (project_id, created_at);
-- Two active runs never share a port.
CREATE UNIQUE INDEX runs_by_active_port ON runs (port)
    WHERE port IS NOT NULL AND state IN ('setting_up', 'starting', 'ready', 'running');
