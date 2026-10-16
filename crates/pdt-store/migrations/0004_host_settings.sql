-- Per-host settings (spec §6) as JSON under a key; today only key 'host'. Never synced between hosts.
CREATE TABLE host_settings (
    key   TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL
);

-- Per-project host overrides: run configuration overrides (TOML text) and the agent credential policy.
CREATE TABLE project_settings (
    project_id        TEXT PRIMARY KEY NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    run_overrides     TEXT,
    agent_credentials TEXT
);
