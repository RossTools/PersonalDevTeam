-- PDT-618: what agents other than the PM may do to a project's documents without asking (pdt_core::policy::
-- AgentDocuments: off | own_ticket | project), and a record of each change an agent made that way.
ALTER TABLE project_settings ADD COLUMN agent_documents TEXT NOT NULL DEFAULT 'off';

CREATE TABLE document_changes (
    id          TEXT PRIMARY KEY NOT NULL,
    document_id TEXT NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
    at          INTEGER NOT NULL,
    tool        TEXT NOT NULL,              -- document_create | doc_edit | sheet_edit | document_link
    summary     TEXT NOT NULL,              -- what it did, for people
    version     TEXT,                       -- the provider's version after it, when it has one
    diff        TEXT,                       -- a text document's unified diff
    agent       TEXT NOT NULL,              -- the agent session (AgentBy JSON: session, persona, its ticket)
    ticket_id   TEXT                        -- the ticket the agent worked on
);
CREATE INDEX document_changes_by_document ON document_changes(document_id, at);
