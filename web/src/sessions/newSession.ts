import type { AgentKind } from '../api/gen/AgentKind';
import type { NewSession } from '../api/gen/NewSession';
import type { PermissionMode } from '../api/gen/PermissionMode';
import type { SessionMode } from '../api/gen/SessionMode';
import { agentLabel, BUILTIN_AGENTS, isAcp, type AcpKind } from '../common/agents';
import type { AcpAgentView } from '../api/gen/AcpAgentView';
import { INITIAL_SIZE } from './status';

/**
 * `Claude`, `Codex` and `Devin` are conversations; the `(terminal)` choices are the agents' own TUIs. An ACP agent's
 * choice is its kind (`acp:<id>`): always a conversation.
 */
export type Choice = 'Shell' | 'Claude' | 'Claude (terminal)' | 'Codex' | 'Codex (terminal)' | 'Devin' | 'Devin (terminal)' | AcpKind;

/** The agent a choice starts (null for a shell). */
export function agentOf(choice: Choice): AgentKind | null {
  if (choice === 'Shell') return null;
  if (isAcp(choice)) return choice;
  return choice.startsWith('Codex') ? 'codex' : choice.startsWith('Devin') ? 'devin' : 'claude';
}

/** What a choice is called on screen: an ACP agent by its label. */
export const choiceText = (choice: Choice, acp: AcpAgentView[] | undefined): string => (isAcp(choice) ? agentLabel(choice, acp) : choice);

/** The ACP agents' choices. */
export const acpChoices = (acp: AcpAgentView[]): Choice[] => acp.map((a) => a.agent as AcpKind);

/** The host's default agent, view and permission modes (Settings → This host → Agents). */
export interface AgentDefaults {
  agent: AgentKind;
  mode: SessionMode;
  /** What a ticket's Start offers first. */
  ticketPermissionMode: PermissionMode;
  /** What a new session offers first. */
  sessionPermissionMode: PermissionMode;
}

/**
 * What the app offers before the host settings load (or when they fail): a Claude conversation; Bypass for a ticket
 * (its agent works on its own branch), Ask for a new session.
 */
export const FALLBACK_DEFAULTS: AgentDefaults = { agent: 'claude', mode: 'conversation', ticketPermissionMode: 'bypass', sessionPermissionMode: 'ask' };

const LABEL: Record<string, 'Claude' | 'Codex' | 'Devin'> = { claude: 'Claude', codex: 'Codex', devin: 'Devin' };

/** The choice that starts `agent` as a conversation or in its terminal (an ACP agent has only its conversation). */
export function choiceFor(agent: AgentKind, mode: SessionMode): Choice {
  if (isAcp(agent)) return agent;
  return mode === 'terminal' ? `${LABEL[agent]} (terminal)` : LABEL[agent];
}

/**
 * Pure: `items` with the default agent's choices first, each agent's default view before its other one; the other
 * agents keep their order and Shell stays last.
 */
export function byDefault<T>(items: T[], choiceOf: (item: T) => Choice, d: Pick<AgentDefaults, 'agent' | 'mode'>): T[] {
  const weight = (item: T) => {
    const c = choiceOf(item);
    const a = agentOf(c);
    if (!a) return 100;
    const otherView = c.endsWith('(terminal)') === (d.mode === 'terminal') ? 0 : 1;
    // ACP agents after the built-in ones, in their own order.
    const rank = isAcp(a) ? BUILTIN_AGENTS.length : BUILTIN_AGENTS.indexOf(a);
    return (a === d.agent ? -1 : rank) * 2 + otherView;
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
