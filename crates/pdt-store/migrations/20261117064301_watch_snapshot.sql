-- Watches of documents (PDT-616): the document's content when its watcher was last told, as JSON (what the next
-- change is diffed against). NULL for tickets and runs.
ALTER TABLE watches ADD COLUMN snapshot TEXT;
