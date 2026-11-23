-- Remote projects (PDT-631). On a remote instance: a project of kind 'remote' and its link to the owner. The owner's
-- public key verifies its commands; this instance's own private key for the link is in the keychain.
CREATE TABLE remote_projects (
    project_id TEXT PRIMARY KEY NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    link_id TEXT NOT NULL UNIQUE,
    owner_project_id TEXT NOT NULL,
    owner_instance_id TEXT NOT NULL,
    owner_name TEXT NOT NULL,
    owner_url TEXT NOT NULL,
    owner_version TEXT NOT NULL,
    owner_public_key TEXT NOT NULL,
    repo_url TEXT,
    paused INTEGER NOT NULL DEFAULT 0,
    last_contact_at INTEGER,
    created_at INTEGER NOT NULL,
    UNIQUE (owner_instance_id, owner_project_id)
);

-- On the owner: the instances that joined one of its projects. The remote instance's public key verifies its
-- reports; the owner's private key for the link is in the keychain.
CREATE TABLE remote_instances (
    link_id TEXT PRIMARY KEY NOT NULL,
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    instance_id TEXT NOT NULL,
    name TEXT NOT NULL,
    url TEXT NOT NULL,
    public_key TEXT NOT NULL,
    version TEXT NOT NULL,
    protocol INTEGER NOT NULL,
    facts TEXT,
    paused INTEGER NOT NULL DEFAULT 0,
    last_seen_at INTEGER,
    last_error TEXT,
    created_at INTEGER NOT NULL,
    UNIQUE (project_id, instance_id)
);
CREATE INDEX remote_instances_by_project ON remote_instances (project_id);
