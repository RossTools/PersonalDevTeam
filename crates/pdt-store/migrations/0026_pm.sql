-- PDT-81 (docs/superpowers/specs/2026-10-21-pm-persona-design.md): the PM's notes and journal (searched with FTS5),
-- its sessions' rotation, and the project policy that lets it act without asking.

CREATE TABLE pm_notes (
    id         TEXT PRIMARY KEY NOT NULL,
    project_id TEXT REFERENCES projects(id) ON DELETE CASCADE,  -- null: host-wide
    kind       TEXT NOT NULL DEFAULT 'note',                    -- note | journal
    title      TEXT NOT NULL,
    body       TEXT NOT NULL DEFAULT '',
    pinned     INTEGER NOT NULL DEFAULT 0,
    session_id TEXT,                                            -- a journal entry's PM session
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE INDEX pm_notes_by_kind ON pm_notes (kind, updated_at);

CREATE VIRTUAL TABLE pm_notes_fts USING fts5 (title, body, content = 'pm_notes', content_rowid = 'rowid');

CREATE TRIGGER pm_notes_ai AFTER INSERT ON pm_notes BEGIN
    INSERT INTO pm_notes_fts (rowid, title, body) VALUES (new.rowid, new.title, new.body);
END;
CREATE TRIGGER pm_notes_ad AFTER DELETE ON pm_notes BEGIN
    INSERT INTO pm_notes_fts (pm_notes_fts, rowid, title, body) VALUES ('delete', old.rowid, old.title, old.body);
END;
CREATE TRIGGER pm_notes_au AFTER UPDATE ON pm_notes BEGIN
    INSERT INTO pm_notes_fts (pm_notes_fts, rowid, title, body) VALUES ('delete', old.rowid, old.title, old.body);
    INSERT INTO pm_notes_fts (rowid, title, body) VALUES (new.rowid, new.title, new.body);
END;

CREATE TABLE pm_sessions (
    session_id TEXT PRIMARY KEY NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    rotation   TEXT NOT NULL DEFAULT 'none',                    -- none | journal | done
    reason     TEXT,                                            -- context | age | user
    created_at INTEGER NOT NULL,
    ended_at   INTEGER
);

ALTER TABLE project_settings ADD COLUMN pm_acts INTEGER NOT NULL DEFAULT 0;
