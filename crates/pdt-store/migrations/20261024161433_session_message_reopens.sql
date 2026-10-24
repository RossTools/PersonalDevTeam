-- A workflow message that reopens a conversation that is not running (stopped, interrupted by a restart) to take it,
-- as the user's own messages do (PDT-144): an agent step's next pass, a ticket comment. Unlike `by_user`, the host's
-- cap on agents running a turn still holds it.
ALTER TABLE session_messages ADD COLUMN reopens INTEGER NOT NULL DEFAULT 0;
