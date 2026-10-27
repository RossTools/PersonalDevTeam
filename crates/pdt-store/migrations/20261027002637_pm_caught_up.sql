-- PDT-167: PM conversations no longer end on a full context or at a day old. Each gets a daily update instead; this
-- is when it was last brought up to date (null: its briefing, at created_at).
ALTER TABLE pm_sessions ADD COLUMN caught_up_at INTEGER;
