-- PDT-46: instructions PDT gives every agent session in the project, kept on the host instead of in the repo.
-- 0017, not 0016: PDT-45 took 0016 on main first.
ALTER TABLE project_settings ADD COLUMN agent_instructions TEXT;
