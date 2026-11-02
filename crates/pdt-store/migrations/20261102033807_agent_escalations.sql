-- PDT-367: "When an agent is stuck" handles a stuck agent step. A step it was handed waits here until one of its nodes
-- acts on it, or PDT falls back to the Inbox; `restarts` counts the fresh sessions a step had.
ALTER TABLE agent_steps ADD COLUMN restarts INTEGER NOT NULL DEFAULT 0;
CREATE TABLE agent_escalations (
    step_id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    why TEXT NOT NULL DEFAULT '',
    at INTEGER NOT NULL
);
