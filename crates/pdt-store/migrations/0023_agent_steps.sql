-- Phase 7b (docs/superpowers/specs/2026-10-20-phase-7b-agent-steps-design.md): personas and their versions, agent
-- steps' session bindings, the Inbox, and Personal projects.

CREATE TABLE personas (
    id          TEXT PRIMARY KEY NOT NULL,
    name        TEXT NOT NULL UNIQUE COLLATE NOCASE,
    role        TEXT NOT NULL DEFAULT '',
    description TEXT NOT NULL DEFAULT '',
    builtin     TEXT UNIQUE,                       -- the built-in template id
    archived    INTEGER NOT NULL DEFAULT 0,
    version     INTEGER NOT NULL,                  -- current
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
);

CREATE TABLE persona_versions (
    persona_id      TEXT NOT NULL REFERENCES personas(id) ON DELETE CASCADE,
    version         INTEGER NOT NULL,
    agent           TEXT NOT NULL,                 -- claude | codex | devin
    model           TEXT,
    instructions    TEXT NOT NULL DEFAULT '',
    permission_mode TEXT,                          -- null: project override, else host default
    tools           TEXT NOT NULL DEFAULT '[]',    -- JSON list of pdt-mcp tools; empty: the default set
    author          TEXT NOT NULL,                 -- user | builtin | agent
    note            TEXT NOT NULL DEFAULT '',
    created_at      INTEGER NOT NULL,
    PRIMARY KEY (persona_id, version)
);

-- Which session an agent step runs in, and what it may answer (pdt-mcp reads it).
CREATE TABLE agent_steps (
    step_id         TEXT PRIMARY KEY NOT NULL REFERENCES workflow_steps(id) ON DELETE CASCADE,
    run_id          TEXT NOT NULL REFERENCES workflow_runs(id) ON DELETE CASCADE,
    node_id         TEXT NOT NULL,
    session_id      TEXT REFERENCES sessions(id) ON DELETE SET NULL,
    persona_id      TEXT REFERENCES personas(id) ON DELETE SET NULL,
    persona_name    TEXT NOT NULL,
    persona_version INTEGER NOT NULL,
    project_id      TEXT,
    ticket_id       TEXT,
    attempt_id      TEXT,
    outcomes        TEXT NOT NULL,                 -- JSON [{id, description}]
    state           TEXT NOT NULL,                 -- working | asking | stalled | done
    turns           INTEGER NOT NULL DEFAULT 0,
    nudges          INTEGER NOT NULL DEFAULT 0,
    max_turns       INTEGER NOT NULL,
    auto_nudges     INTEGER NOT NULL,
    outcome         TEXT,
    summary         TEXT,
    comments        TEXT,                          -- JSON
    by_user         INTEGER NOT NULL DEFAULT 0,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL
);
CREATE INDEX agent_steps_by_session ON agent_steps (session_id, state);
CREATE INDEX agent_steps_by_ticket ON agent_steps (ticket_id, created_at);

CREATE TABLE inbox_items (
    id          TEXT PRIMARY KEY NOT NULL,
    kind        TEXT NOT NULL,                     -- question | gate | stalled | loop_limit
    grp         TEXT NOT NULL,                     -- question | gate | stalled | loop_limit | accept
    status      TEXT NOT NULL,                     -- open | resolved | closed
    project_id  TEXT,
    ticket_id   TEXT,
    attempt_id  TEXT,
    session_id  TEXT,
    run_id      TEXT,
    step_id     TEXT,
    persona_id  TEXT,
    title       TEXT NOT NULL,
    body        TEXT NOT NULL DEFAULT '{}',
    choices     TEXT NOT NULL DEFAULT '[]',
    resolution  TEXT,
    created_at  INTEGER NOT NULL,
    resolved_at INTEGER
);
CREATE INDEX inbox_items_open ON inbox_items (status, created_at);
CREATE UNIQUE INDEX one_open_item_per_step ON inbox_items (step_id, kind) WHERE status = 'open' AND step_id IS NOT NULL;

ALTER TABLE sessions ADD COLUMN persona_id TEXT;
ALTER TABLE sessions ADD COLUMN persona_version INTEGER;
ALTER TABLE sessions ADD COLUMN step_id TEXT;

ALTER TABLE projects ADD COLUMN kind TEXT NOT NULL DEFAULT 'repo';  -- repo | personal

ALTER TABLE project_settings ADD COLUMN agent_permission_mode TEXT;  -- overrides personas' modes
