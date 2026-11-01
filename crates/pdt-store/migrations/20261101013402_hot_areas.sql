-- PDT-329: overlap-aware pickup. The areas "Pick up tickets" keeps to one ticket in flight at a time (a JSON list), and
-- whether a ticket without areas may start alongside hot work (and the other way round).
ALTER TABLE project_settings ADD COLUMN hot_areas TEXT NOT NULL DEFAULT '[]';
ALTER TABLE project_settings ADD COLUMN untagged_alongside_hot INTEGER NOT NULL DEFAULT 1;
