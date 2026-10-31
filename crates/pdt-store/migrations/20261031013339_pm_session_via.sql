-- Where a PM conversation is held besides PDT (PDT-304): `discord` for one the user talks to through the Discord bot's DMs.
ALTER TABLE pm_sessions ADD COLUMN via TEXT;
