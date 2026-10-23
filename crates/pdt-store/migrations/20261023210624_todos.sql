-- The user's todo list (PDT-125). A todo outlives the project or ticket it was tied to (the link goes).
CREATE TABLE todos (
    id TEXT PRIMARY KEY NOT NULL,
    project_id TEXT REFERENCES projects(id) ON DELETE SET NULL,
    ticket_id TEXT REFERENCES tickets(id) ON DELETE SET NULL,
    title TEXT NOT NULL,
    done INTEGER NOT NULL DEFAULT 0,
    position REAL NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    done_at INTEGER
);
CREATE INDEX todos_by_position ON todos (done, position);
CREATE INDEX todos_by_project ON todos (project_id);
