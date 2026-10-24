-- A message on its way to a conversation (PDT-129): `sending` until the agent echoes it (then `told`), with the id of
-- the conversation's pending message that carries it. The user's own conversation messages get a row too, so a
-- restart that loses one can say so.
ALTER TABLE session_messages ADD COLUMN pending_id TEXT;
CREATE INDEX session_messages_sending ON session_messages (pending_id) WHERE status = 'sending';
