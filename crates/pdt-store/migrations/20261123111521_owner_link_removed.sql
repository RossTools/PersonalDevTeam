-- On a remote instance: when the owner was found to have removed this instance from the project while the two
-- couldn't reach each other (PDT-702). NULL: as far as this instance knows, it is still joined.
ALTER TABLE remote_projects ADD COLUMN removed_at INTEGER;
