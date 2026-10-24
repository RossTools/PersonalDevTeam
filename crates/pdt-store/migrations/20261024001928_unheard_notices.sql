-- PDT-139: notices nobody could be told become Inbox items, one per attempt and notice key.
ALTER TABLE inbox_items ADD COLUMN dedupe_key TEXT;
CREATE INDEX inbox_items_dedupe ON inbox_items (dedupe_key) WHERE dedupe_key IS NOT NULL;

-- A workflow's merge refused for something the attempt's agent can fix: PR notices fixes its later failures.
ALTER TABLE attempts ADD COLUMN merge_refused INTEGER NOT NULL DEFAULT 0;
