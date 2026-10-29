-- PDT-242: CI runs belong to a CI provider and their ids are text. The runs of `ci.run_failed` / `ci.run_passed`
-- events logged before get their event's provider, their id as text, and `name` for `workflow`.
UPDATE events
SET payload = json_set(payload, '$.runs', (
    SELECT json_group_array(json_object(
        'provider', COALESCE(json_extract(events.payload, '$.provider'), 'github_actions'),
        'id', CAST(json_extract(r.value, '$.id') AS TEXT),
        'attempt', json_extract(r.value, '$.attempt'),
        'name', COALESCE(json_extract(r.value, '$.workflow'), json_extract(r.value, '$.name'), ''),
        'state', json_extract(r.value, '$.state'),
        'url', json_extract(r.value, '$.url'),
        'failedJobs', json(COALESCE(json_extract(r.value, '$.failedJobs'), '[]'))
    ))
    FROM json_each(events.payload, '$.runs') AS r
))
WHERE type IN ('ci.run_failed', 'ci.run_passed')
  AND json_valid(payload)
  AND json_type(payload, '$.runs') = 'array'
  AND EXISTS (SELECT 1 FROM json_each(events.payload, '$.runs') AS r WHERE json_type(r.value, '$.id') != 'text');
