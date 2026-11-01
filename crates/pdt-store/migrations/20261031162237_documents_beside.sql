-- Documents open beside a conversation (PDT-317): the tabs of an agent or PM conversation's side panel, in order. The
-- user opens them, or the agent does with a tool; the newest focused one is the tab shown.
CREATE TABLE documents_beside (
    session_id  TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    document_id TEXT NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
    position    INTEGER NOT NULL,
    actor       TEXT NOT NULL,              -- user | agent: who opened it
    opened_at   INTEGER NOT NULL,
    focused_at  INTEGER NOT NULL,
    PRIMARY KEY (session_id, document_id)
);
CREATE INDEX documents_beside_by_document ON documents_beside (document_id);
