-- PDT-46: instructions PDT gives every agent session in the project, kept on the host instead of in the repo.
ALTER TABLE project_settings ADD COLUMN agent_instructions TEXT;
