import { useQuery } from '@tanstack/react-query';
import { api } from '../api/client';
import type { AgentKind } from '../api/gen/AgentKind';
import type { HostChecks } from '../api/gen/HostChecks';

export const AGENT_KINDS: AgentKind[] = ['claude', 'codex', 'devin'];

/** What the host checks say about one agent: `missing` (hide it), `logged_out` (offer it with a hint) or `ready`. */
export type AgentAvailability = 'ready' | 'logged_out' | 'missing';

/** Pure: an agent's availability from the checks; everything is `ready` until they have loaded. */
export function availabilityOf(checks: HostChecks | undefined, agent: AgentKind): AgentAvailability {
  const c = checks?.[agent];
  if (!c) return 'ready';
  return !c.found ? 'missing' : c.ok ? 'ready' : 'logged_out';
}

/** The hint shown beside an installed agent that is not logged in (the session is where you log in). */
export const NOT_LOGGED_IN = 'not logged in';

/**
 * Which agents this host has, from `GET /api/host/checks` (the query Host settings' "Run checks again" refreshes).
 * Never waits on the checks: until they load, or when they fail, every agent is offered.
 */
export function useAgentAvailability() {
  const checks = useQuery({ queryKey: ['host-checks'], queryFn: api.hostChecks, staleTime: 60_000, retry: false });
  const of = (agent: AgentKind) => availabilityOf(checks.data, agent);
  return {
    of,
    offered: (agent: AgentKind) => of(agent) !== 'missing',
    /** "not logged in" for an installed agent that is logged out, else undefined. */
    hint: (agent: AgentKind) => (of(agent) === 'logged_out' ? NOT_LOGGED_IN : undefined),
  };
}
