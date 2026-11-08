// This host: its settings, setup, checks and update, each one query shared by every screen that shows it.
import { useQuery, type QueryClient } from '@tanstack/react-query';
import { api } from '../api/client';
import type { AgentKind } from '../api/gen/AgentKind';
import type { HostSettingsView } from '../api/gen/HostSettingsView';

export const hostKeys = {
  settings: ['host-settings'] as const,
  setup: ['host-setup'] as const,
  checks: ['host-checks'] as const,
  update: ['host-update'] as const,
  whoami: ['host-whoami'] as const,
  health: ['health'] as const,
  bugReportPreview: ['bug-report-preview'] as const,
  folders: ['host-folders'] as const,
  folder: (path: string, nearest: boolean, hidden: boolean) => ['host-folders', path, nearest, hidden] as const,
  agentCapabilities: (agent: AgentKind) => ['agent-capabilities', agent] as const,
};

/**
 * The host settings (Settings → This host, the agents, their defaults). They change through this UI, which sets or
 * invalidates them, so a mount within a minute doesn't refetch; a failure shows at once (the page offers Retry).
 */
export function useHostSettings() {
  return useQuery({ queryKey: hostKeys.settings, queryFn: api.hostSettings, staleTime: 60_000, retry: false });
}

/** After a write that answers with the host settings as they are now. */
export function setHostSettings(qc: QueryClient, view: HostSettingsView) {
  qc.setQueryData(hostKeys.settings, view);
}

/** What the host can run (the agents, git, the tools), checked again when a write may change it. */
export function useHostChecks() {
  return useQuery({ queryKey: hostKeys.checks, queryFn: api.hostChecks, staleTime: 60_000, retry: false });
}

/** An agent's capabilities (its models, modes and efforts), which change only with the agent. */
export function useAgentCapabilities(agent: AgentKind) {
  return useQuery({ queryKey: hostKeys.agentCapabilities(agent), queryFn: () => api.agentCapabilities(agent), staleTime: 60_000 });
}
