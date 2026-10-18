-- Tickets (tickets spec §3). Phase 4a uses tickets, labels, links and events; attempts, areas, `waiting` and
-- sessions.attempt_id are created now so Phases 4b and 4c add no schema churn to these tables.

-- Ticket numbers are never reused, even after deletes.
ALTER TABLE projects ADD COLUMN next_ticket_number INTEGER NOT NULL DEFAULT 1;

CREATE TABLE tickets (
    id          TEXT PRIMARY KEY NOT NULL,
    -- A stable integer key for the full-text index (a TEXT-keyed table's rowid may change on VACUUM).
    seq         INTEGER NOT NULL UNIQUE,
    project_id  TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    number      INTEGER NOT NULL,
    title       TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    stage       TEXT NOT NULL,                -- draft | ready | in_progress | done | wont_do
    priority    TEXT NOT NULL DEFAULT 'none', -- none | low | medium | high | urgent
    rank        TEXT NOT NULL,                -- fractional index: lexicographic order = backlog order
    waiting     INTEGER NOT NULL DEFAULT 0,
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL,
    closed_at   INTEGER,
    UNIQUE (project_id, number)
);
CREATE INDEX tickets_by_rank ON tickets (project_id, rank);
CREATE INDEX tickets_by_updated ON tickets (project_id, updated_at);

CREATE TABLE ticket_labels (
    ticket_id TEXT NOT NULL REFERENCES tickets(id) ON DELETE CASCADE,
    label     TEXT NOT NULL,
    PRIMARY KEY (ticket_id, label)
);
CREATE INDEX ticket_labels_by_label ON ticket_labels (label);

CREATE TABLE ticket_areas (
    ticket_id TEXT NOT NULL REFERENCES tickets(id) ON DELETE CASCADE,
    area      TEXT NOT NULL,
    PRIMARY KEY (ticket_id, area)
);

CREATE TABLE ticket_links (
    from_id    TEXT NOT NULL REFERENCES tickets(id) ON DELETE CASCADE,  -- blocks: from blocks to
    to_id      TEXT NOT NULL REFERENCES tickets(id) ON DELETE CASCADE,
    kind       TEXT NOT NULL,                                           -- blocks | relates
    created_at INTEGER NOT NULL,
    PRIMARY KEY (from_id, to_id, kind),
    CHECK (from_id <> to_id)
);
CREATE INDEX ticket_links_by_to ON ticket_links (to_id, kind);

CREATE TABLE attempts (
    id          TEXT PRIMARY KEY NOT NULL,
    ticket_id   TEXT NOT NULL REFERENCES tickets(id) ON DELETE CASCADE,
    worktree_id TEXT REFERENCES worktrees(id) ON DELETE SET NULL,
    branch      TEXT NOT NULL,
    base_branch TEXT NOT NULL,
    base_commit TEXT NOT NULL,
    state       TEXT NOT NULL,                -- active | merged | abandoned
    created_at  INTEGER NOT NULL,
    ended_at    INTEGER
);
CREATE UNIQUE INDEX one_active_attempt ON attempts (ticket_id) WHERE state = 'active';
CREATE INDEX attempts_by_ticket ON attempts (ticket_id, created_at);

-- The ticket's timeline; comments are events.
CREATE TABLE ticket_events (
    id         TEXT PRIMARY KEY NOT NULL,
    ticket_id  TEXT NOT NULL REFERENCES tickets(id) ON DELETE CASCADE,
    kind       TEXT NOT NULL,
    actor      TEXT NOT NULL,                 -- user | system
    body       TEXT,
    data       TEXT,                          -- JSON
    created_at INTEGER NOT NULL
);
CREATE INDEX ticket_events_by_ticket ON ticket_events (ticket_id, created_at);

-- Full-text search over titles and descriptions, kept in sync by triggers.
CREATE VIRTUAL TABLE tickets_fts USING fts5(
    title, description,
    content = 'tickets', content_rowid = 'seq',
    tokenize = 'unicode61 remove_diacritics 2', prefix = '2 3'
);
CREATE TRIGGER tickets_fts_insert AFTER INSERT ON tickets BEGIN
    INSERT INTO tickets_fts (rowid, title, description) VALUES (new.seq, new.title, new.description);
END;
CREATE TRIGGER tickets_fts_delete AFTER DELETE ON tickets BEGIN
    INSERT INTO tickets_fts (tickets_fts, rowid, title, description)
    VALUES ('delete', old.seq, old.title, old.description);
END;
CREATE TRIGGER tickets_fts_update AFTER UPDATE OF title, description ON tickets BEGIN
    INSERT INTO tickets_fts (tickets_fts, rowid, title, description)
    VALUES ('delete', old.seq, old.title, old.description);
    INSERT INTO tickets_fts (rowid, title, description) VALUES (new.seq, new.title, new.description);
END;

ALTER TABLE sessions ADD COLUMN attempt_id TEXT REFERENCES attempts(id) ON DELETE SET NULL;

-- Workspace tabs gain a ticket kind: exactly one of session, run, ticket, each with at most one tab. SQLite can't
-- change a CHECK in place, so the table is rebuilt (nothing references workspace_tabs, so dropping it is safe with
-- foreign keys on).
CREATE TABLE workspace_tabs_new (
    id           TEXT PRIMARY KEY NOT NULL,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    session_id   TEXT UNIQUE REFERENCES sessions(id) ON DELETE CASCADE,
    run_id       TEXT UNIQUE REFERENCES runs(id) ON DELETE CASCADE,
    ticket_id    TEXT UNIQUE REFERENCES tickets(id) ON DELETE CASCADE,
    position     INTEGER NOT NULL,
    created_at   INTEGER NOT NULL,
    CHECK ((session_id IS NOT NULL) + (run_id IS NOT NULL) + (ticket_id IS NOT NULL) = 1)
);
INSERT INTO workspace_tabs_new (id, workspace_id, session_id, run_id, ticket_id, position, created_at)
SELECT id, workspace_id, session_id, run_id, NULL, position, created_at FROM workspace_tabs;
DROP TABLE workspace_tabs;
ALTER TABLE workspace_tabs_new RENAME TO workspace_tabs;
CREATE INDEX workspace_tabs_by_workspace ON workspace_tabs (workspace_id, position);
