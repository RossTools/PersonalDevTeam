-- Documents (PDT-314): design docs, plans and spreadsheets kept by a provider (`local`, or a plugin's such as
-- `google`). PDT keeps this record of each and its links to projects and tickets; the content stays with the provider.
CREATE TABLE documents (
    id           TEXT PRIMARY KEY NOT NULL,
    title        TEXT NOT NULL,
    kind         TEXT NOT NULL,              -- doc | sheet
    provider     TEXT NOT NULL,
    provider_ref TEXT NOT NULL,              -- the provider's file id or path
    url          TEXT,
    actor        TEXT NOT NULL,              -- user | agent
    agent        TEXT,                       -- the agent session that added it (AgentBy JSON)
    created_at   INTEGER NOT NULL,
    updated_at   INTEGER NOT NULL,
    UNIQUE (provider, provider_ref)
);
CREATE INDEX documents_by_updated ON documents (updated_at);

CREATE TABLE document_projects (
    document_id TEXT NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
    project_id  TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    PRIMARY KEY (document_id, project_id)
);
CREATE INDEX document_projects_by_project ON document_projects (project_id);

CREATE TABLE document_tickets (
    document_id TEXT NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
    ticket_id   TEXT NOT NULL REFERENCES tickets(id) ON DELETE CASCADE,
    PRIMARY KEY (document_id, ticket_id)
);
CREATE INDEX document_tickets_by_ticket ON document_tickets (ticket_id);

-- A project's documents settings: its default provider and local documents folder (none: the host's).
CREATE TABLE project_document_settings (
    project_id TEXT PRIMARY KEY NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    provider   TEXT,
    folder     TEXT
);
