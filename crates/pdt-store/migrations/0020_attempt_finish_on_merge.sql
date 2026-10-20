-- Merge… on a pull request can ask PDT to finish the ticket once GitHub merges it (PDT-75): move it to Done, remove
-- the worktree (and its branch with finish_delete_branch) and the workspace Start made for it.
ALTER TABLE attempts ADD COLUMN finish_on_merge INTEGER NOT NULL DEFAULT 0;
ALTER TABLE attempts ADD COLUMN finish_delete_branch INTEGER NOT NULL DEFAULT 0;
