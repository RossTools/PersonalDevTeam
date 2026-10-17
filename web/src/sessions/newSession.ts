import type { NewSession } from '../api/gen/NewSession';
import { INITIAL_SIZE } from './status';

export type Choice = 'Shell' | 'Claude' | 'Codex';

/** The create request for a shell or an agent in one place; `bypass` skips the agent's permission prompts. */
export function newSession(projectId: string, worktreeId: string | null, choice: Choice, bypass: boolean): NewSession {
  const base = { projectId, ...(worktreeId ? { worktreeId } : {}) };
  return choice === 'Shell'
    ? { ...base, kind: 'shell', ...INITIAL_SIZE }
    : {
        ...base,
        kind: 'agent_tui',
        agent: choice === 'Claude' ? 'claude' : 'codex',
        // Only sent when armed, so a normal launch stays a plain payload.
        ...(bypass ? { bypassPermissions: true } : {}),
        ...INITIAL_SIZE,
      };
}
