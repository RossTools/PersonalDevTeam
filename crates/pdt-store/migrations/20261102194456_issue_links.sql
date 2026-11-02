-- PDT-481: issues linked by hand, and plain links. A link records how it was made (created: PDT filed the issue from
-- the ticket; imported: the ticket came from the issue; manual: someone linked an issue that was there), whether Done
-- closes the issue (always for created and imported links; a manual link only when the user turns it on), and the
-- issue's title and state as the tracker last said.

ALTER TABLE ticket_links_external ADD COLUMN origin TEXT NOT NULL DEFAULT 'imported';   -- created | imported | manual
ALTER TABLE ticket_links_external ADD COLUMN close_on_done INTEGER NOT NULL DEFAULT 1;
ALTER TABLE ticket_links_external ADD COLUMN title TEXT;
ALTER TABLE ticket_links_external ADD COLUMN state TEXT;                                 -- open | closed
ALTER TABLE ticket_links_external ADD COLUMN checked_at INTEGER;                         -- when they were read

-- Until now only an import made a ticket external with its link; a link on any other ticket is an issue PDT filed.
UPDATE ticket_links_external SET origin = 'created'
 WHERE ticket_id IN (SELECT id FROM tickets WHERE source <> 'external');

CREATE TABLE ticket_web_links (
    id         TEXT PRIMARY KEY,
    ticket_id  TEXT NOT NULL REFERENCES tickets(id) ON DELETE CASCADE,
    url        TEXT NOT NULL,
    label      TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    UNIQUE (ticket_id, url)
);
