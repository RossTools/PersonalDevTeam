-- A todo's notes (PDT-227): Markdown, written in its open view; empty when it has none.
ALTER TABLE todos ADD COLUMN body TEXT NOT NULL DEFAULT '';
