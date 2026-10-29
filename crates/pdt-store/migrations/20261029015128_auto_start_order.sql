-- PDT-218: the order auto-start takes a project's candidates in: `priority` (most urgent first, then rank; the default,
-- existing projects too) or `rank`.

ALTER TABLE project_settings ADD COLUMN auto_start_order TEXT NOT NULL DEFAULT 'priority';
