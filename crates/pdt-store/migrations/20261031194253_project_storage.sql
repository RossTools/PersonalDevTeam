-- A project's automatic worktree cleanup on this host (pdt_core::storage::StorageSettings, JSON).
ALTER TABLE project_settings ADD COLUMN storage TEXT NOT NULL DEFAULT '{}';
