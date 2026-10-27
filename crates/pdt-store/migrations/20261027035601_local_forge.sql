-- The local forge (PDT-178): change requests and CI runs of projects with no remote, kept by PDT itself.
CREATE TABLE local_changes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    -- Numbered per project from 1.
    number INTEGER NOT NULL,
    branch TEXT NOT NULL,
    base_branch TEXT NOT NULL,
    title TEXT NOT NULL,
    -- open, merged, closed
    state TEXT NOT NULL,
    -- Set to merge once CI passes.
    auto_merge INTEGER NOT NULL DEFAULT 0,
    -- The branch head last seen (while open), or the one merged.
    head_sha TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    merged_at INTEGER,
    UNIQUE (project_id, number)
);
CREATE INDEX local_changes_by_branch ON local_changes (project_id, branch);

CREATE TABLE local_ci_runs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    change_id INTEGER NOT NULL REFERENCES local_changes(id) ON DELETE CASCADE,
    head_sha TEXT NOT NULL,
    command TEXT NOT NULL,
    -- running, passed, failed, cancelled
    state TEXT NOT NULL,
    exit_code INTEGER,
    created_at INTEGER NOT NULL,
    ended_at INTEGER
);
CREATE INDEX local_ci_runs_by_change ON local_ci_runs (change_id);
