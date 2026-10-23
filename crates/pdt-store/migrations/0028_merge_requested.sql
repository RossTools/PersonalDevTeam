-- PDT-104: an attempt whose pull request was set to merge (Merge…, `attempt.merge`, or auto-merge / the merge queue
-- seen on GitHub), so PR notices gets it merging again after a failure (sticky).
ALTER TABLE attempts ADD COLUMN merge_requested INTEGER NOT NULL DEFAULT 0;
