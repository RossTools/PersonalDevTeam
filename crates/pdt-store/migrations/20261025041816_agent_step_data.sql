-- PDT-154: what an agent hands in with its outcome beyond the summary (submit_outcome's data: an issue it duplicates,
-- a ticket key), for the workflow's later steps (`{{steps.<id>.data.<field>}}`).
ALTER TABLE agent_steps ADD COLUMN data TEXT;
