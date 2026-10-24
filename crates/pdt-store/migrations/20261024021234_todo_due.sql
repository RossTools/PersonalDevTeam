-- A todo's due day (PDT-140): `YYYY-MM-DD` in the user's local day, an optional `HH:MM`, and the due key its reminder
-- was last made for (one per todo and due date, even across restarts).
ALTER TABLE todos ADD COLUMN due_on TEXT;
ALTER TABLE todos ADD COLUMN due_time TEXT;
ALTER TABLE todos ADD COLUMN reminded_key TEXT;
