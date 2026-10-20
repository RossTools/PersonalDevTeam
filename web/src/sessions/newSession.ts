import type { AgentKind } from '../api/gen/AgentKind';
import type { NewSession } from '../api/gen/NewSession';
import type { PermissionMode } from '../api/gen/PermissionMode';
import type { SessionMode } from '../api/gen/SessionMode';
import { INITIAL_SIZE } from './status';

/** `Claude`, `Codex` and `Devin` are conversations; the `(terminal)` choices are the agents' own TUIs. */
export type Choice = 'Shell' | 'Claude' | 'Claude (terminal)' | 'Codex' | 'Codex (terminal)' | 'Devin' | 'Devin (terminal)';

/** The agent a choice starts (null for a shell). */
export function agentOf(choice: Choice): AgentKind | null {
  if (choice === 'Shell') return null;
  return choice.startsWith('Codex') ? 'codex' : choice.startsWith('Devin') ? 'devin' : 'claude';
}

/** The host's default agent and view (Settings → This host → Agents). */
export interface AgentDefaults {
  agent: AgentKind;
  mode: SessionMode;
}

/** What the app offers before the host settings load (or when they fail): a Claude conversation. */
export const FALLBACK_DEFAULTS: AgentDefaults = { agent: 'claude', mode: 'conversation' };

const LABEL: Record<AgentKind, 'Claude' | 'Codex' | 'Devin'> = { claude: 'Claude', codex: 'Codex', devin: 'Devin' };

/** The choice that starts `agent` as a conversation or in its terminal. */
export function choiceFor(agent: AgentKind, mode: SessionMode): Choice {
  return mode === 'terminal' ? `${LABEL[agent]} (terminal)` : LABEL[agent];
}

/**
 * Pure: `items` with the default agent's choices first, each agent's default view before its other one; the other
 * agents keep their order and Shell stays last.
 */
export function byDefault<T>(items: T[], choiceOf: (item: T) => Choice, d: AgentDefaults): T[] {
  const order: AgentKind[] = ['claude', 'codex', 'devin'];
  const weight = (item: T) => {
    const c = choiceOf(item);
    const a = agentOf(c);
    if (!a) return 100;
    const otherView = c.endsWith('(terminal)') === (d.mode === 'terminal') ? 0 : 1;
    return (a === d.agent ? -1 : order.indexOf(a)) * 2 + otherView;
  };
  return items.map((item, i) => ({ item, i, w: weight(item) })).sort((x, y) => x.w - y.w || x.i - y.i).map((x) => x.item);
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
