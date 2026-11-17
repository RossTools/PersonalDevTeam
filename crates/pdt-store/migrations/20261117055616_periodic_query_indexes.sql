-- Indexes for the queries pdtd's loops run every few seconds, which had come to scan whole tables as history grew
-- (PDT-614).
-- An event's key alone: finding a timeline event's copy in the log (starts left half-done, undoing a start).
CREATE INDEX events_by_key ON events (key);
-- Timeline events of one kind (the overlaps loop reads every `overlap` event).
CREATE INDEX ticket_events_by_kind ON ticket_events (kind, created_at);
-- Runs and steps by status: the partial indexes on `status IN (…)` were never used, since a query asking for
-- `status = 'waiting'` doesn't repeat that list.
DROP INDEX workflow_runs_active;
CREATE INDEX workflow_runs_by_status ON workflow_runs (status, wake_at);
DROP INDEX workflow_steps_waiting;
CREATE INDEX workflow_steps_by_status ON workflow_steps (status);
-- Pruning finished runs: the old ones, and for each, its child runs (`parent_run_id` is set to NULL; without an index
-- SQLite scanned every run for each one deleted).
CREATE INDEX workflow_runs_by_ended ON workflow_runs (ended_at);
CREATE INDEX workflow_runs_by_parent ON workflow_runs (parent_run_id) WHERE parent_run_id IS NOT NULL;
-- Agent steps not yet done (who owns each ticket's step, read on every board refresh), and a run's.
CREATE INDEX agent_steps_open ON agent_steps (created_at) WHERE state != 'done';
CREATE INDEX agent_steps_by_run ON agent_steps (run_id, node_id);
