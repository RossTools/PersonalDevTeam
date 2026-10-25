-- PDT-151: an automated merge of a pull request waits for green CI on its head (on unless the project turns it off).
ALTER TABLE project_settings ADD COLUMN require_green_ci INTEGER NOT NULL DEFAULT 1;
