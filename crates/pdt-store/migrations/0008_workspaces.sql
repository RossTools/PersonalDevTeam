-- Workspaces and tabs (workspaces spec §2, §5). A project workspace has project_id set; free-form ones don't.
CREATE TABLE workspaces (
    id         TEXT PRIMARY KEY NOT NULL,
    name       TEXT NOT NULL,
    project_id TEXT UNIQUE REFERENCES projects(id) ON DELETE CASCADE,
    position   INTEGER NOT NULL,
    created_at INTEGER NOT NULL
);

-- A tab shows one session or one run; each has at most one tab.
CREATE TABLE workspace_tabs (
    id           TEXT PRIMARY KEY NOT NULL,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    session_id   TEXT UNIQUE REFERENCES sessions(id) ON DELETE CASCADE,
    run_id       TEXT UNIQUE REFERENCES runs(id) ON DELETE CASCADE,
    position     INTEGER NOT NULL,
    created_at   INTEGER NOT NULL,
    CHECK ((session_id IS NULL) <> (run_id IS NULL))
);
CREATE INDEX workspace_tabs_by_workspace ON workspace_tabs (workspace_id, position);

-- Every existing project gets its workspace, in project order.
INSERT INTO workspaces (id, name, project_id, position, created_at)
SELECT lower(hex(randomblob(16))), name, id,
       ROW_NUMBER() OVER (ORDER BY created_at, id) - 1, created_at
FROM projects;

-- ...holding a tab for each of its sessions and active runs: the main checkout first, then worktrees in the order
-- they were made, then the items in the order they were made.
INSERT INTO workspace_tabs (id, workspace_id, session_id, run_id, position, created_at)
SELECT lower(hex(randomblob(16))), w.id, i.session_id, i.run_id,
       ROW_NUMBER() OVER (
           PARTITION BY w.id
           ORDER BY i.worktree_id IS NOT NULL, COALESCE(wt.created_at, 0), wt.id, i.created_at, i.item_id
       ) - 1,
       i.created_at
FROM (
    SELECT project_id, worktree_id, id AS session_id, NULL AS run_id, id AS item_id, created_at FROM sessions
    UNION ALL
    SELECT project_id, worktree_id, NULL, id, id, created_at FROM runs
    WHERE state IN ('setting_up', 'starting', 'ready', 'running')
) AS i
JOIN workspaces w ON w.project_id = i.project_id
LEFT JOIN worktrees wt ON wt.id = i.worktree_id;
