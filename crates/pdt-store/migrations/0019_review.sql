-- Review (Phase 6; docs/superpowers/specs/2026-10-19-review-design.md §3): line comments on a worktree's changes,
-- and changes tabs.

CREATE TABLE review_comments (
    id          TEXT PRIMARY KEY NOT NULL,
    worktree_id TEXT NOT NULL REFERENCES worktrees(id) ON DELETE CASCADE,
    parent_id   TEXT REFERENCES review_comments(id) ON DELETE CASCADE,  -- a reply's thread root
    path        TEXT NOT NULL,           -- checkout-relative, '/'-separated (a reply copies its root's anchor)
    side        TEXT NOT NULL,           -- old | new
    line        INTEGER NOT NULL,        -- 1-based first line on that side
    end_line    INTEGER NOT NULL,        -- last line (= line for one line)
    line_text   TEXT NOT NULL,           -- the anchored lines when the comment was made
    body        TEXT NOT NULL,
    author      TEXT NOT NULL DEFAULT 'user',
    sent_at     INTEGER,                 -- null: a draft
    session_id  TEXT,                    -- where it was sent (no FK: the session may be removed)
    resolved_at INTEGER,                 -- roots only
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
);
CREATE INDEX review_comments_by_worktree ON review_comments (worktree_id, path);

-- Workspace tabs gain a changes kind (a worktree's review). Rebuilt as in 0010: a CHECK can't change in place.
CREATE TABLE workspace_tabs_new (
    id           TEXT PRIMARY KEY NOT NULL,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    session_id   TEXT UNIQUE REFERENCES sessions(id) ON DELETE CASCADE,
    run_id       TEXT UNIQUE REFERENCES runs(id) ON DELETE CASCADE,
    ticket_id    TEXT UNIQUE REFERENCES tickets(id) ON DELETE CASCADE,
    worktree_id  TEXT UNIQUE REFERENCES worktrees(id) ON DELETE CASCADE,
    position     INTEGER NOT NULL,
    created_at   INTEGER NOT NULL,
    CHECK ((session_id IS NOT NULL) + (run_id IS NOT NULL) + (ticket_id IS NOT NULL) + (worktree_id IS NOT NULL) = 1)
);
INSERT INTO workspace_tabs_new (id, workspace_id, session_id, run_id, ticket_id, worktree_id, position, created_at)
SELECT id, workspace_id, session_id, run_id, ticket_id, NULL, position, created_at FROM workspace_tabs;
DROP TABLE workspace_tabs;
ALTER TABLE workspace_tabs_new RENAME TO workspace_tabs;
CREATE INDEX workspace_tabs_by_workspace ON workspace_tabs (workspace_id, position);
