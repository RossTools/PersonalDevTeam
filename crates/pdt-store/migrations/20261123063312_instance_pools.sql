-- Pickup per instance (PDT-636). On the owner: each joined instance's cap (the project's setting for it) and the cap
-- the instance reported for the project. On a remote instance: its own cap for a remote project.
ALTER TABLE remote_instances ADD COLUMN cap INTEGER;
ALTER TABLE remote_instances ADD COLUMN remote_cap INTEGER;
ALTER TABLE remote_projects ADD COLUMN cap INTEGER;
