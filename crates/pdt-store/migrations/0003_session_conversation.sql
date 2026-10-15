-- external_session_id: the agent's own conversation id (Codex cannot be given one up front, so it is found in
-- Codex's rollout files). A conversation belongs to at most one session.
-- fresh_started_at: when a Codex session was last launched WITHOUT a conversation to resume (ms). Codex stamps its
-- conversation with its process start time, so this is what ties a rollout file to its session. NULL for rows from
-- before it was recorded, which are never bound.
ALTER TABLE sessions ADD COLUMN external_session_id TEXT;
ALTER TABLE sessions ADD COLUMN fresh_started_at INTEGER;
CREATE UNIQUE INDEX sessions_by_external_id ON sessions (external_session_id) WHERE external_session_id IS NOT NULL;
