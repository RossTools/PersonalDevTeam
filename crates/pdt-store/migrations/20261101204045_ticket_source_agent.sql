-- PDT-363: the agent session that made a ticket (its `created` event's `agent`, JSON), for "Created by Coder · from
-- PDT-345" wherever the ticket shows; and tickets an agent session other than the PM made are `agent`, not
-- `workflow`.
ALTER TABLE tickets ADD COLUMN created_by TEXT;

UPDATE tickets SET created_by = (
    SELECT json_extract(e.data, '$.agent') FROM ticket_events e
    WHERE e.ticket_id = tickets.id AND e.kind = 'created' AND json_extract(e.data, '$.agent.sessionId') IS NOT NULL
    ORDER BY e.created_at LIMIT 1
);

UPDATE tickets SET source = 'agent' WHERE source = 'workflow' AND created_by IS NOT NULL;
