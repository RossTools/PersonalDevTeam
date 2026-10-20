import type { AgentKind } from '../api/gen/AgentKind';
import type { NewSession } from '../api/gen/NewSession';
import type { PermissionMode } from '../api/gen/PermissionMode';
import { INITIAL_SIZE } from './status';

/** `Claude`, `Codex` and `Devin` are conversations; the `(terminal)` choices are the agents' own TUIs. */
export type Choice = 'Shell' | 'Claude' | 'Claude (terminal)' | 'Codex' | 'Codex (terminal)' | 'Devin' | 'Devin (terminal)';

/** The agent a choice starts (null for a shell). */
export function agentOf(choice: Choice): AgentKind | null {
  if (choice === 'Shell') return null;
  return choice.startsWith('Codex') ? 'codex' : choice.startsWith('Devin') ? 'devin' : 'claude';
}

export interface NewSessionOptions {
  /** Terminal agents: skip the agent's permission prompts. */
  bypass?: boolean;
  /** Conversations: the permission mode (`ask` when absent). */
  permissionMode?: PermissionMode;
}

/** The create request for a shell, a terminal agent or a conversation in one place. */
export function newSession(projectId: string, worktreeId: string | null, choice: Choice, opts: NewSessionOptions = {}): NewSession {
  const base = { projectId, ...(worktreeId ? { worktreeId } : {}) };
  if (choice === 'Shell') return { ...base, kind: 'shell', ...INITIAL_SIZE };
  const agent = agentOf(choice)!;
  if (!choice.endsWith('(terminal)')) {
    return {
      ...base,
      kind: 'agent_tui',
      agent,
      mode: 'conversation',
      // Only sent when it isn't the default, so a normal launch stays a plain payload.
      ...(opts.permissionMode && opts.permissionMode !== 'ask' ? { permissionMode: opts.permissionMode } : {}),
      ...INITIAL_SIZE,
    };
  }
  return {
    ...base,
    kind: 'agent_tui',
    agent,
    mode: 'terminal',
    ...(opts.bypass ? { bypassPermissions: true } : {}),
    ...INITIAL_SIZE,
  };
}
