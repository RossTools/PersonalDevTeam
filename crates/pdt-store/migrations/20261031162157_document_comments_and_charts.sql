-- Local documents (PDT-315): the comments PDT keeps on a document (anchored to a heading, some text, or a sheet's
-- cell or range; a reply has a parent), and the charts PDT draws on a local sheet (their specs as JSON).
CREATE TABLE document_comments (
    id          TEXT PRIMARY KEY NOT NULL,
    document_id TEXT NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
    parent_id   TEXT REFERENCES document_comments(id) ON DELETE CASCADE,
    anchor      TEXT,                        -- CommentAnchor JSON (top-level comments only)
    author      TEXT NOT NULL,
    actor       TEXT NOT NULL,               -- user | agent
    body        TEXT NOT NULL,
    resolved    INTEGER NOT NULL DEFAULT 0,
    created_at  INTEGER NOT NULL
);
CREATE INDEX document_comments_by_document ON document_comments (document_id, created_at);

CREATE TABLE document_charts (
    document_id TEXT NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
    chart_id    TEXT NOT NULL,
    spec        TEXT NOT NULL,               -- ChartSpec JSON
    position    INTEGER NOT NULL,
    PRIMARY KEY (document_id, chart_id)
);
