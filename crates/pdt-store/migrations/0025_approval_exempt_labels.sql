-- PDT-99: labels whose tickets merge automatically without the approval `require_approval` asks for.

ALTER TABLE project_settings ADD COLUMN approval_exempt_labels TEXT NOT NULL DEFAULT '[]';  -- JSON list of labels
