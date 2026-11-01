-- PDT-363: tickets an agent session other than the PM made are `agent`, not `workflow` (their `created` event names
-- the session).
UPDATE tickets SET source = 'agent'
WHERE source = 'workflow'
  AND EXISTS (
    SELECT 1 FROM ticket_events e
    WHERE e.ticket_id = tickets.id AND e.kind = 'created' AND json_extract(e.data, '$.agent.sessionId') IS NOT NULL
  );
