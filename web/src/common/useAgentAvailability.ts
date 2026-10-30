import { useQuery } from '@tanstack/react-query';
import { api } from '../api/client';
import type { AgentKind } from '../api/gen/AgentKind';
import type { HostChecks } from '../api/gen/HostChecks';
import type { ToolCheck } from '../api/gen/ToolCheck';
import { isAcp, useAgentsOff } from './agents';

export { BUILTIN_AGENTS as AGENT_KINDS } from './agents';

/**
 * What the host checks say about one agent: `missing` (hide it), `logged_out` (offer it with a hint) or `ready`; `off`
 * when it is switched off in Settings → Agents (hide it, PDT-270).
 */
export type AgentAvailability = 'ready' | 'logged_out' | 'missing' | 'off';

/** Pure: an agent's check (an ACP agent's under `acp`), when the checks have one. */
export const checkOf = (checks: HostChecks | undefined, agent: AgentKind): ToolCheck | undefined =>
  isAcp(agent) ? checks?.acp?.[agent] : checks?.[agent];

/** Pure: an agent's availability from the checks and the agents switched off; `ready` until the checks have loaded. */
export function availabilityOf(checks: HostChecks | undefined, agent: AgentKind, off: readonly AgentKind[] = []): AgentAvailability {
  if (off.includes(agent)) return 'off';
  const c = checkOf(checks, agent);
  if (!c) return 'ready';
  return !c.found ? 'missing' : c.ok ? 'ready' : 'logged_out';
}

/** The hint shown beside an installed agent that is not logged in (the session is where you log in). */
export const NOT_LOGGED_IN = 'not logged in';

/**
 * Which agents this host has, from `GET /api/host/checks` (the query Host settings' "Run checks again" refreshes),
 * less the ones switched off. Never waits on the checks: until they load, or when they fail, every agent switched on
 * is offered.
 */
export function useAgentAvailability() {
  const checks = useQuery({ queryKey: ['host-checks'], queryFn: api.hostChecks, staleTime: 60_000, retry: false });
  const off = useAgentsOff();
  const of = (agent: AgentKind) => availabilityOf(checks.data, agent, off);
  return {
    of,
    offered: (agent: AgentKind) => of(agent) !== 'missing' && of(agent) !== 'off',
    /** "not logged in" for an installed agent that is logged out, else undefined. */
    hint: (agent: AgentKind) => (of(agent) === 'logged_out' ? NOT_LOGGED_IN : undefined),
  };
}
