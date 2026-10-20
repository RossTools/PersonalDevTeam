import { useQuery } from '@tanstack/react-query';
import { api } from '../api/client';
import { FALLBACK_DEFAULTS, type AgentDefaults } from '../sessions/newSession';

/**
 * The host's default agent, view and permission modes (Settings → This host → Agents), from the `['host-settings']` query the
 * settings page updates when they are saved. Never waits: until it loads, or when it fails, a Claude conversation.
 */
export function useAgentDefaults(): AgentDefaults {
  const q = useQuery({ queryKey: ['host-settings'], queryFn: api.hostSettings, staleTime: 60_000, retry: false });
  const s = q.data?.settings;
  return s
    ? {
        agent: s.defaultAgent,
        mode: s.defaultSessionMode,
        ticketPermissionMode: s.ticketPermissionMode ?? FALLBACK_DEFAULTS.ticketPermissionMode,
        sessionPermissionMode: s.sessionPermissionMode ?? FALLBACK_DEFAULTS.sessionPermissionMode,
      }
    : FALLBACK_DEFAULTS;
}
