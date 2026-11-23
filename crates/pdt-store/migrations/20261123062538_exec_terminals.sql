-- On a remote instance: which of the sessions it runs for an owner are terminals (PDT-633). A terminal's PTY ends with
-- this instance, so its row is ended when the instance starts again.
ALTER TABLE exec_sessions ADD COLUMN terminal INTEGER NOT NULL DEFAULT 0;
