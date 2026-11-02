-- PDT-377: a project's board settings on this host (pdt_core::board::BoardSettings, JSON): its Done window.
ALTER TABLE project_settings ADD COLUMN board TEXT NOT NULL DEFAULT '{}';
