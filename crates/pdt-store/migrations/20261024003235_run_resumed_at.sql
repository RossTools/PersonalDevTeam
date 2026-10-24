-- When Retry or Resume last set a run going again (PDT-131): its maxRuntimeMs counts from then, not from its start.
ALTER TABLE workflow_runs ADD COLUMN resumed_at INTEGER;
