-- Set once merge detection has seen the attempt's branch hold commits that are not in its base (PDT-45). Until then a
-- head that is in the base only caught up with it (fast-forwarded to a newer main), so it is not called merged.
ALTER TABLE attempts ADD COLUMN own_commits INTEGER NOT NULL DEFAULT 0;
