-- Whether the user's first message may still name a session (PDT-739): set only for an untitled session that has
-- had no message yet, cleared by that message or any rename. Sessions made before this keep their titles.
ALTER TABLE sessions ADD COLUMN auto_title INTEGER NOT NULL DEFAULT 0;

-- The maintainer's "PDT" PM conversation, renamed after a later message by the rule this replaces.
UPDATE sessions SET title = 'PDT' WHERE id = '01a1a1c3-af82-72c4-b85d-07b67ce0c88a' AND title = 'Cut a release';
