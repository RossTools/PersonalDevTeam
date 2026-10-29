import { useQuery } from '@tanstack/react-query';
import { api } from '../api/client';
import type { AcpAgentView } from '../api/gen/AcpAgentView';
import type { AgentKind } from '../api/gen/AgentKind';

/** The agents built into PDT, in the order the app lists them. */
export const BUILTIN_AGENTS: AgentKind[] = ['claude', 'codex', 'devin'];
export type BuiltinAgent = 'claude' | 'codex' | 'devin';

/** An ACP agent's kind (PDT-234): a built-in profile or one the user added in Settings → Agents. */
export type AcpKind = `acp:${string}`;

export const isAcp = (agent: string | null | undefined): agent is AcpKind => !!agent?.startsWith('acp:');

const BUILTIN_LABEL: Record<string, string> = { claude: 'Claude', codex: 'Codex', devin: 'Devin' };

/** Pure: an agent's name: a built-in's, else the ACP agent's label from `acp`, else its id. */
export function agentLabel(agent: AgentKind, acp: Pick<AcpAgentView, 'agent' | 'label'>[] | undefined): string {
  if (!isAcp(agent)) return BUILTIN_LABEL[agent];
  return acp?.find((a) => a.agent === agent)?.label ?? agent.slice('acp:'.length);
}

/** The host's ACP agents (from the host settings; none until they load). */
export function useAcpAgents(): AcpAgentView[] {
  const q = useQuery({ queryKey: ['host-settings'], queryFn: api.hostSettings, staleTime: 60_000, retry: false });
  return q.data?.acpAgents ?? [];
}

/** Every agent this host has (the built-ins, then its ACP agents) and their names. */
export function useAgents() {
  const acp = useAcpAgents();
  return {
    acp,
    all: [...BUILTIN_AGENTS, ...acp.map((a) => a.agent)],
    label: (agent: AgentKind) => agentLabel(agent, acp),
  };
}
