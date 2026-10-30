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

const useHostSettings = () => useQuery({ queryKey: ['host-settings'], queryFn: api.hostSettings, staleTime: 60_000, retry: false });

/** The host's ACP agents (from the host settings; none until they load). */
export function useAcpAgents(): AcpAgentView[] {
  return useHostSettings().data?.acpAgents ?? [];
}

/** The agents switched off in Settings → Agents (PDT-270; none until the host settings load). */
export function useAgentsOff(): AgentKind[] {
  return useHostSettings().data?.settings.disabledAgents ?? [];
}

/**
 * The agents this host offers (the built-ins, then its ACP agents; a switched-off one left out, PDT-270) and their
 * names. `on` tells whether one is switched on.
 */
export function useAgents() {
  const host = useHostSettings().data;
  const acp = host?.acpAgents ?? [];
  const off = host?.settings.disabledAgents ?? [];
  return {
    acp,
    all: host?.enabledAgents ?? [...BUILTIN_AGENTS, ...acp.map((a) => a.agent)],
    on: (agent: AgentKind) => !off.includes(agent),
    label: (agent: AgentKind) => agentLabel(agent, acp),
  };
}
