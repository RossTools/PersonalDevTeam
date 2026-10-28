-- Where a ticket came from (PDT-201): user, pm, workflow or external. Tickets made before default to user.
ALTER TABLE tickets ADD COLUMN source TEXT NOT NULL DEFAULT 'user';
-- When the user let an external ticket in (moved it to To do, started it or labelled it agentic): the Todo gate is open.
ALTER TABLE tickets ADD COLUMN admitted_at INTEGER;
