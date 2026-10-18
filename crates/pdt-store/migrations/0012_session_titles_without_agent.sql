-- Session titles no longer start with the agent ("Claude · ", "Codex · ", "Shell · "): the UI shows the kind with
-- an icon instead (PDT-20). Strips the prefix PDT itself added from existing titles, once.
UPDATE sessions SET title = substr(title, 10) WHERE kind = 'agent_tui' AND agent = 'claude' AND title LIKE 'Claude · _%';
UPDATE sessions SET title = substr(title, 9) WHERE kind = 'agent_tui' AND agent = 'codex' AND title LIKE 'Codex · _%';
UPDATE sessions SET title = substr(title, 9) WHERE kind = 'shell' AND title LIKE 'Shell · _%';
