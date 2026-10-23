-- Indexes for the housekeeping that bounds what PDT keeps (PDT-134): conversation events whose protocol frames are
-- still stored, and finished outbox messages.
CREATE INDEX session_events_with_raw ON session_events (ts) WHERE raw IS NOT NULL;
CREATE INDEX session_messages_resolved ON session_messages (resolved_at) WHERE status != 'waiting';
