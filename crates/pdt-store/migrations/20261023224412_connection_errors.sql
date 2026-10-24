-- PDT-130: why a sensor's last use of a connection failed (a revoked token, the service down), shown in Settings →
-- Connections; cleared when a use works again or the connection is saved.
ALTER TABLE connections ADD COLUMN last_error TEXT;
ALTER TABLE connections ADD COLUMN last_error_at INTEGER;
