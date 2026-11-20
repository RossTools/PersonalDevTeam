-- A conversation opens on its latest events and pages back from there: both read by place.
CREATE INDEX session_events_by_pos ON session_events (session_id, pos);
