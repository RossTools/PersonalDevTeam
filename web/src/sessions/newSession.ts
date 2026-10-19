import type { NewSession } from '../api/gen/NewSession';
import type { PermissionMode } from '../api/gen/PermissionMode';
import { INITIAL_SIZE } from './status';

/** `Claude` is a conversation; `Claude (terminal)` is Claude's own TUI; Codex is terminal-only for now. */
export type Choice = 'Shell' | 'Claude' | 'Claude (terminal)' | 'Codex';

export interface NewSessionOptions {
  /** Terminal agents: skip the agent's permission prompts. */
  bypass?: boolean;
  /** Claude conversations: the permission mode (`ask` when absent). */
  permissionMode?: PermissionMode;
}

/** The create request for a shell, a terminal agent or a Claude conversation in one place. */
export function newSession(projectId: string, worktreeId: string | null, choice: Choice, opts: NewSessionOptions = {}): NewSession {
  const base = { projectId, ...(worktreeId ? { worktreeId } : {}) };
  if (choice === 'Shell') return { ...base, kind: 'shell', ...INITIAL_SIZE };
  if (choice === 'Claude') {
    return {
      ...base,
      kind: 'agent_tui',
      agent: 'claude',
      mode: 'conversation',
      // Only sent when it isn't the default, so a normal launch stays a plain payload.
      ...(opts.permissionMode && opts.permissionMode !== 'ask' ? { permissionMode: opts.permissionMode } : {}),
      ...INITIAL_SIZE,
    };
  }
  return {
    ...base,
    kind: 'agent_tui',
    agent: choice === 'Codex' ? 'codex' : 'claude',
    ...(choice === 'Claude (terminal)' ? { mode: 'terminal' as const } : {}),
    ...(opts.bypass ? { bypassPermissions: true } : {}),
    ...INITIAL_SIZE,
  };
}
