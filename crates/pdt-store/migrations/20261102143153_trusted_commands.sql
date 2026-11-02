-- PDT-405: the commands from a project's run configuration the user approved, by fingerprint
-- (`pdt_core::command_trust`). Projects added before this carry the `legacy` mark instead: their main checkout's
-- commands are trusted the first time they are checked.
CREATE TABLE trusted_commands (
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    fingerprint TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (project_id, fingerprint)
);
INSERT INTO trusted_commands (project_id, fingerprint, created_at)
    SELECT id, 'legacy', CAST(strftime('%s', 'now') AS INTEGER) * 1000 FROM projects;
