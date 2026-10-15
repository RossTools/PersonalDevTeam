CREATE TABLE worktrees (
    id          TEXT PRIMARY KEY NOT NULL,
    project_id  TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    branch      TEXT NOT NULL,
    path        TEXT NOT NULL UNIQUE,
    base_commit TEXT NOT NULL,
    created_at  INTEGER NOT NULL,
    UNIQUE (project_id, branch)
);

CREATE TABLE sessions (
    id          TEXT PRIMARY KEY NOT NULL,
    project_id  TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    worktree_id TEXT REFERENCES worktrees(id) ON DELETE CASCADE,
    kind        TEXT NOT NULL,
    agent       TEXT,
    title       TEXT NOT NULL,
    cwd         TEXT NOT NULL,
    status      TEXT NOT NULL,
    exit_code   INTEGER,
    created_at  INTEGER NOT NULL,
    ended_at    INTEGER
);

CREATE INDEX sessions_by_project ON sessions (project_id, created_at);
