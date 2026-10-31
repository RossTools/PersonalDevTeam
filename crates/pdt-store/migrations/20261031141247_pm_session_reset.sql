-- A PM conversation the user reset (PDT-308): the one it was reset from, and the list position it keeps (the old one's
-- last activity, until it has activity of its own).
ALTER TABLE pm_sessions ADD COLUMN reset_from TEXT;
ALTER TABLE pm_sessions ADD COLUMN listed_at INTEGER;
