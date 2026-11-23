-- Remote attempts (PDT-632). On the owner: the remote instance (its instance id) a session or worktree runs on, the
-- project's placement rules, and how far it has read each joined instance's session events.
ALTER TABLE sessions ADD COLUMN instance_id TEXT;
ALTER TABLE worktrees ADD COLUMN instance_id TEXT;
ALTER TABLE project_settings ADD COLUMN placement TEXT;
ALTER TABLE remote_instances ADD COLUMN events_boot TEXT;
ALTER TABLE remote_instances ADD COLUMN events_after INTEGER NOT NULL DEFAULT 0;

-- On a remote instance: where it keeps a remote project's checkout, and the sessions it runs for the owner (their
-- rows are the owner's; these say only what the relay needs: the link, the folder and the session's pdt-mcp token).
ALTER TABLE remote_projects ADD COLUMN checkout_path TEXT;
CREATE TABLE exec_sessions (
    session_id TEXT PRIMARY KEY NOT NULL,
    link_id TEXT NOT NULL REFERENCES remote_projects(link_id) ON DELETE CASCADE,
    agent TEXT NOT NULL,
    title TEXT NOT NULL,
    ticket_key TEXT,
    cwd TEXT NOT NULL,
    token TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    ended_at INTEGER
);
CREATE INDEX exec_sessions_by_link ON exec_sessions (link_id);
