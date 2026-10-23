-- PDT-85 (docs/superpowers/specs/2026-10-20-phase-7e-integrations-design.md §6): tickets linked to issues in other
-- trackers (Jira), and a project's Jira and Buildkite settings on this host.

CREATE TABLE ticket_links_external (
    ticket_id     TEXT NOT NULL REFERENCES tickets(id) ON DELETE CASCADE,
    system        TEXT NOT NULL,                  -- jira
    connection_id TEXT NOT NULL,                  -- the connection the issue is read through
    key           TEXT NOT NULL,                  -- ABC-123
    url           TEXT NOT NULL,
    created_at    INTEGER NOT NULL,
    PRIMARY KEY (ticket_id, system, connection_id, key),
    UNIQUE (system, connection_id, key)           -- an issue is linked to one ticket: never imported twice
);

ALTER TABLE project_settings ADD COLUMN jira_connection TEXT;                       -- a jira connection's name
ALTER TABLE project_settings ADD COLUMN jira_scope TEXT NOT NULL DEFAULT '';        -- JQL, e.g. project = ABC
ALTER TABLE project_settings ADD COLUMN jira_done TEXT NOT NULL DEFAULT 'Done';     -- the done transition or status
ALTER TABLE project_settings ADD COLUMN buildkite_connection TEXT;                  -- a buildkite connection's name
ALTER TABLE project_settings ADD COLUMN buildkite_pipelines TEXT NOT NULL DEFAULT '[]';  -- JSON list of slugs
