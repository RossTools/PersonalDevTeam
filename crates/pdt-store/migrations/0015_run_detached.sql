-- Detached runs (PDT-32): the command runs outside pdtd's terminals and survives pdtd restarts. `process` names its
-- processes (a Windows job object or a unix process group) while they may be alive, so the next pdtd can find them.
-- 0015, not 0014: another branch (PDT-2) already uses 0014.
ALTER TABLE runs ADD COLUMN detached INTEGER NOT NULL DEFAULT 0;
ALTER TABLE runs ADD COLUMN process TEXT;
