-- PDT-709: what a local document was before an agent's unasked change, so the user can restore it: the file's bytes
-- and, for a sheet, its charts (JSON: [[sheet, chart], ...]). Kept for the newest few changes of each document.
ALTER TABLE document_changes ADD COLUMN backup BLOB;
ALTER TABLE document_changes ADD COLUMN backup_charts TEXT;
