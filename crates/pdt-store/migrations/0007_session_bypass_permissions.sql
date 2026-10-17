-- Whether an agent session was launched with its permission prompts bypassed, so restart/auto-resume keep it.
ALTER TABLE sessions ADD COLUMN bypass_permissions INTEGER NOT NULL DEFAULT 0;
