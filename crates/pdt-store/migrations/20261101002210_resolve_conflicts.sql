-- PDT-328: when an attempt's conflicts with its base are resolved: at merge time (merge_time) or at once (immediately).
ALTER TABLE project_settings ADD COLUMN resolve_conflicts TEXT NOT NULL DEFAULT 'merge_time';
