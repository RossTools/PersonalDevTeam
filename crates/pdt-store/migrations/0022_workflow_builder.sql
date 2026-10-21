-- Phase 7d (docs/superpowers/specs/2026-10-20-phase-7d-builder-design.md): proposed versions and the user's templates.

-- saved | proposed (an agent's change waiting for review) | discarded
ALTER TABLE workflow_versions ADD COLUMN status TEXT NOT NULL DEFAULT 'saved';
CREATE INDEX workflow_versions_proposed ON workflow_versions (workflow_id) WHERE status = 'proposed';

CREATE TABLE workflow_templates (
    id          TEXT PRIMARY KEY NOT NULL,
    name        TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    params      TEXT NOT NULL,                 -- JSON: [TemplateParam]
    doc         TEXT NOT NULL,                 -- JSON: WorkflowDoc with {{params.x}}
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
);
