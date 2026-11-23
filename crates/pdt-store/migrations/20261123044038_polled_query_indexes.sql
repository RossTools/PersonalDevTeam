-- More indexes for queries pdtd's polls and loops run, which scanned tables that grow with history (PDT-652).
-- An attempt's sessions (a ticket's sessions go through its attempts; deleting an attempt sets theirs to NULL).
CREATE INDEX sessions_by_attempt ON sessions (attempt_id) WHERE attempt_id IS NOT NULL;
-- Sessions by status (the notifier, outbox and idle checks read the running ones).
CREATE INDEX sessions_by_status ON sessions (status, created_at);
-- The attempt of a workspace and of a worktree.
CREATE INDEX attempts_by_workspace ON attempts (workspace_id) WHERE workspace_id IS NOT NULL;
CREATE INDEX attempts_by_worktree ON attempts (worktree_id) WHERE worktree_id IS NOT NULL;
-- Runs started since a time (the triage caps), and the newest runs.
CREATE INDEX workflow_runs_by_started ON workflow_runs (started_at);
-- Unread notifications (the Inbox's dot counts the updates): few, while most are read or cleared.
CREATE INDEX notifications_unread ON notifications (needs_you, project_id)
    WHERE read_at IS NULL AND cleared_at IS NULL AND dismissed_at IS NULL;
-- A session's notifications (marking them read).
CREATE INDEX notifications_by_session ON notifications (session_id) WHERE session_id IS NOT NULL;
-- Resolved Inbox items (pruning the old ones).
CREATE INDEX inbox_items_resolved ON inbox_items (resolved_at) WHERE status != 'open';
-- Active runs on this host.
CREATE INDEX runs_active ON runs (created_at) WHERE state IN ('setting_up', 'starting', 'ready', 'running');
