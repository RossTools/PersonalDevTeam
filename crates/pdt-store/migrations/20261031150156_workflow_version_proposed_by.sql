-- Who proposed a workflow version (PDT-320): the agent's persona or session, shown in the Inbox's proposal item.
ALTER TABLE workflow_versions ADD COLUMN proposed_by TEXT;
