-- PDT-594: a project's branch naming on this host (pdt_core::branch_name::BranchSettings, JSON: its template and
-- types), and whether PDT named an attempt's branch from it (1), or was given the name (0); NULL for attempts from
-- before, which are known by their `pdt-<KEY>` prefix.
ALTER TABLE project_settings ADD COLUMN branch_names TEXT NOT NULL DEFAULT '{}';
ALTER TABLE attempts ADD COLUMN branch_named INTEGER;
