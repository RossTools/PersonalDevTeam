-- external_session_id: the agent's own conversation id (Codex cannot be given one up front, so it is found in
-- Codex's rollout files). started_at: when the session's process was last spawned (ms).
ALTER TABLE sessions ADD COLUMN external_session_id TEXT;
ALTER TABLE sessions ADD COLUMN started_at INTEGER;
