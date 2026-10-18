-- The branch head merge detection last saw (tickets spec §6), so a branch merged and then deleted is still
-- recognised as merged.
ALTER TABLE attempts ADD COLUMN last_head TEXT;
