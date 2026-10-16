-- Agent activity (remote-host spec §5): NULL for shells and ended sessions.
ALTER TABLE sessions ADD COLUMN activity TEXT;
-- When the session last became idle or needs_you (ms); unseen while it is later than last_viewed_at.
ALTER TABLE sessions ADD COLUMN attention_at INTEGER;
ALTER TABLE sessions ADD COLUMN last_viewed_at INTEGER;
