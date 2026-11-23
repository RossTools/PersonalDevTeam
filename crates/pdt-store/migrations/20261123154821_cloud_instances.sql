-- Cloud instances (PDT-638), on the owner: a joined instance that is a VM at a cloud provider, started on demand and
-- stopped when idle. `connection` names the connection with the provider's credentials; `machine` the VM there. The
-- rest is what the owner's check keeps between rounds.
CREATE TABLE cloud_instances (
    link_id TEXT PRIMARY KEY NOT NULL REFERENCES remote_instances(link_id) ON DELETE CASCADE,
    connection TEXT NOT NULL,
    machine TEXT NOT NULL,
    idle_minutes INTEGER NOT NULL,
    hourly_rate REAL NOT NULL DEFAULT 0,
    power TEXT,
    checked_at INTEGER,
    last_busy_at INTEGER,
    start_requested_at INTEGER,
    stop_requested_at INTEGER,
    last_error TEXT,
    created_at INTEGER NOT NULL
);

-- The cost meter: running time and its cost per instance and UTC month (`2026-11`), at the rate set while it ran.
CREATE TABLE cloud_usage (
    link_id TEXT NOT NULL REFERENCES remote_instances(link_id) ON DELETE CASCADE,
    month TEXT NOT NULL,
    running_ms INTEGER NOT NULL DEFAULT 0,
    cost REAL NOT NULL DEFAULT 0,
    PRIMARY KEY (link_id, month)
);
