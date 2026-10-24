-- PDT-142: an agent step whose session waits on its background tasks is neither nudged nor stalled, up to a cap.
ALTER TABLE agent_steps ADD COLUMN background_since INTEGER;
ALTER TABLE agent_steps ADD COLUMN background TEXT;
ALTER TABLE agent_steps ADD COLUMN background_wait_ms INTEGER NOT NULL DEFAULT 1800000;
