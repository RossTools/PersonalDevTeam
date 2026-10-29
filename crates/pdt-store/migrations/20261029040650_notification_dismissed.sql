-- PDT-224: the user cleared an Update from the Inbox. It stays hidden, and its row stays live while its cause lasts,
-- so the same cause doesn't raise it again.
ALTER TABLE notifications ADD COLUMN dismissed_at INTEGER;
