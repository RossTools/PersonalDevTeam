-- Phase 7c (docs/superpowers/specs/2026-10-20-phase-7c-pipeline-design.md): a ticket's pipeline stage, project
-- policies, and which outbox messages the user sent (never held by the host's cap on agents running a turn).

ALTER TABLE tickets ADD COLUMN pipeline_stage TEXT;                              -- refine | code | review | qa | accept

ALTER TABLE project_settings ADD COLUMN merge_by TEXT NOT NULL DEFAULT 'agents';  -- agents | user | external
ALTER TABLE project_settings ADD COLUMN require_approval INTEGER NOT NULL DEFAULT 0;
ALTER TABLE project_settings ADD COLUMN max_running_attempts INTEGER;             -- null: no cap
ALTER TABLE project_settings ADD COLUMN auto_start TEXT NOT NULL DEFAULT 'off';   -- off | todo | ready

ALTER TABLE session_messages ADD COLUMN by_user INTEGER NOT NULL DEFAULT 0;
