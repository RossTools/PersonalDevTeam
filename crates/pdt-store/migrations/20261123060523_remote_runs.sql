-- Runs and tests on a remote instance (PDT-634). On the owner: the remote instance (its instance id) a run runs on,
-- when its attempt's worktree is there.
ALTER TABLE runs ADD COLUMN instance_id TEXT;
