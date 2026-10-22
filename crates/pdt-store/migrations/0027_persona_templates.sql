-- PDT-100: the built-in template PDT last applied to a built-in persona (PersonaSetup as JSON), so a newer template
-- can update each field the user left as it was. NULL: not recorded yet (the newest version PDT wrote is it).
ALTER TABLE personas ADD COLUMN builtin_template TEXT;
