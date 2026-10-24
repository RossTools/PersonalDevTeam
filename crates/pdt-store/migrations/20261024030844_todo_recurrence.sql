-- Recurring todos (PDT-141): the rule (`Recurrence` as JSON) and the todo checking this one off made, so it makes at
-- most one, even when it is checked off again.
ALTER TABLE todos ADD COLUMN recurrence TEXT;
ALTER TABLE todos ADD COLUMN next_id TEXT;
