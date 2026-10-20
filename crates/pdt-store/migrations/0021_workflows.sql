-- Phase 7a (docs/superpowers/specs/2026-10-20-phase-7a-engine-design.md): the event log, workflow documents and
-- versions, durable runs and steps, per-workflow state, the session outbox and connections.

-- Typed, keyed events. `key` dedupes: a second event of a type with the same key is dropped.
CREATE TABLE events (
    seq          INTEGER PRIMARY KEY AUTOINCREMENT,
    id           TEXT NOT NULL UNIQUE,
    type         TEXT NOT NULL,
    key          TEXT NOT NULL,
    project_id   TEXT,
    ticket_id    TEXT,
    attempt_id   TEXT,
    session_id   TEXT,
    source       TEXT NOT NULL,
    payload      TEXT NOT NULL,                 -- JSON
    created_at   INTEGER NOT NULL,
    processed_at INTEGER,                       -- matched to triggers by the engine
    UNIQUE (type, key)
);
CREATE INDEX events_unprocessed ON events (seq) WHERE processed_at IS NULL;
CREATE INDEX events_by_type ON events (type, seq);
CREATE INDEX events_by_ticket ON events (ticket_id, seq);
CREATE INDEX events_by_created ON events (created_at);

CREATE TABLE workflows (
    id          TEXT PRIMARY KEY NOT NULL,
    name        TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    scope       TEXT NOT NULL,                 -- project | host | personal
    project_id  TEXT REFERENCES projects(id) ON DELETE CASCADE,
    enabled     INTEGER NOT NULL DEFAULT 0,
    builtin     TEXT UNIQUE,                   -- the built-in template it was installed from
    source      TEXT NOT NULL DEFAULT 'host',  -- host | repo
    repo_file   TEXT,                          -- .pdt/workflows/<name>.json (source = repo)
    version     INTEGER NOT NULL,              -- current
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL,
    CHECK ((scope = 'project') = (project_id IS NOT NULL))
);
CREATE UNIQUE INDEX workflows_by_repo_file ON workflows (project_id, repo_file) WHERE repo_file IS NOT NULL;

CREATE TABLE workflow_versions (
    workflow_id TEXT NOT NULL REFERENCES workflows(id) ON DELETE CASCADE,
    version     INTEGER NOT NULL,
    doc         TEXT NOT NULL,                 -- JSON
    author      TEXT NOT NULL,                 -- user | builtin | repo | agent
    note        TEXT NOT NULL DEFAULT '',
    created_at  INTEGER NOT NULL,
    PRIMARY KEY (workflow_id, version)
);

CREATE TABLE workflow_runs (
    id               TEXT PRIMARY KEY NOT NULL,
    workflow_id      TEXT NOT NULL REFERENCES workflows(id) ON DELETE CASCADE,
    version          INTEGER NOT NULL,
    status           TEXT NOT NULL,            -- running | waiting | succeeded | quiet | failed | cancelled
    run_key          TEXT NOT NULL,
    trigger_event_id TEXT,
    trigger_node     TEXT NOT NULL,
    trigger          TEXT NOT NULL,            -- JSON: a copy of the event payload or manual input
    project_id       TEXT,
    ticket_id        TEXT,
    attempt_id       TEXT,
    parent_run_id    TEXT REFERENCES workflow_runs(id) ON DELETE SET NULL,
    parent_step_id   TEXT,
    error            TEXT,
    output           TEXT,                     -- JSON
    wake_at          INTEGER,
    started_at       INTEGER NOT NULL,
    ended_at         INTEGER,
    UNIQUE (workflow_id, run_key)
);
CREATE INDEX workflow_runs_active ON workflow_runs (status) WHERE status IN ('running', 'waiting');
CREATE INDEX workflow_runs_by_workflow ON workflow_runs (workflow_id, started_at);
CREATE INDEX workflow_runs_by_ticket ON workflow_runs (ticket_id, started_at);

CREATE TABLE workflow_steps (
    id         TEXT PRIMARY KEY NOT NULL,
    run_id     TEXT NOT NULL REFERENCES workflow_runs(id) ON DELETE CASCADE,
    seq        INTEGER NOT NULL,
    node_id    TEXT NOT NULL,
    kind       TEXT NOT NULL,
    status     TEXT NOT NULL,                  -- pending | running | waiting | succeeded | failed | skipped
    input      TEXT,                           -- JSON
    output     TEXT,                           -- JSON
    port       TEXT,
    attempts   INTEGER NOT NULL DEFAULT 0,
    error      TEXT,
    wake_at    INTEGER,
    wait       TEXT,                           -- JSON: what it waits for
    started_at INTEGER,
    ended_at   INTEGER,
    UNIQUE (run_id, seq)
);
CREATE INDEX workflow_steps_waiting ON workflow_steps (status) WHERE status IN ('pending', 'running', 'waiting');

CREATE TABLE workflow_state (
    workflow_id TEXT NOT NULL REFERENCES workflows(id) ON DELETE CASCADE,
    key         TEXT NOT NULL,
    value       TEXT NOT NULL,                 -- JSON
    updated_at  INTEGER NOT NULL,
    PRIMARY KEY (workflow_id, key)
);

-- Messages for agent sessions (`session.send`), delivered by the idle-and-quiet rule; one per step.
CREATE TABLE session_messages (
    id          TEXT PRIMARY KEY NOT NULL,
    attempt_id  TEXT,                          -- to the attempt's newest running agent session
    session_id  TEXT,                          -- or to this session
    ticket_id   TEXT,
    dedupe      TEXT,                          -- per attempt: a key already sent is not sent again
    text        TEXT NOT NULL,
    timeline    TEXT,                          -- JSON: the timeline record to write ({kind, data})
    run_id      TEXT,
    step_id     TEXT UNIQUE,
    status      TEXT NOT NULL,                 -- waiting | told | untold
    told_session TEXT,
    created_at  INTEGER NOT NULL,
    resolved_at INTEGER
);
CREATE INDEX session_messages_waiting ON session_messages (created_at) WHERE status = 'waiting';
CREATE INDEX session_messages_by_attempt ON session_messages (attempt_id, dedupe);

CREATE TABLE connections (
    id         TEXT PRIMARY KEY NOT NULL,
    name       TEXT NOT NULL UNIQUE,
    kind       TEXT NOT NULL,
    url        TEXT NOT NULL DEFAULT '',
    config     TEXT NOT NULL DEFAULT '{}',     -- JSON, nothing secret
    has_secret INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
