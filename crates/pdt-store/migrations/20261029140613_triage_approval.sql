-- PDT-225: tickets triage drafts wait for the user's approval in the Inbox (inbox) or not (off).
ALTER TABLE project_settings ADD COLUMN triage_approval TEXT NOT NULL DEFAULT 'inbox';
