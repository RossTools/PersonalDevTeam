-- New installs start tickets in Ask (PDT-562); an install from before keeps Bypass, the old default, which it may
-- never have stored: settings saved before the field existed lack it, and an install that never saved any has no row.
UPDATE host_settings SET value = json_set(value, '$.ticketPermissionMode', 'bypass')
 WHERE key = 'host' AND json_valid(value) AND json_type(value, '$.ticketPermissionMode') IS NULL;

-- Only the one field: the rest still reads from the profile's defaults.
INSERT INTO host_settings (key, value)
SELECT 'host', '{"ticketPermissionMode":"bypass"}'
 WHERE NOT EXISTS (SELECT 1 FROM host_settings WHERE key = 'host') AND EXISTS (SELECT 1 FROM projects);

-- Get started's permissions step counts as done there: the mode it had is the one it keeps.
UPDATE host_settings SET value = json_set(value, '$.permissionsConfirmed', json('true'))
 WHERE key = 'setup' AND json_valid(value) AND EXISTS (SELECT 1 FROM host_settings WHERE key = 'host');
