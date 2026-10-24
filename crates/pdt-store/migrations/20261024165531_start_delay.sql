-- PDT-146: auto-start takes a ticket only once it has gone this long unchanged (0: at once).
ALTER TABLE project_settings ADD COLUMN start_delay_seconds INTEGER NOT NULL DEFAULT 120;
