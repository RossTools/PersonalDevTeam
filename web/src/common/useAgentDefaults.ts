import { FALLBACK_DEFAULTS, type AgentDefaults } from '../sessions/newSession';
import { useHostSettings } from '../host/queries';

/**
 * The host's default agent, view and permission modes (Settings → This host → Agents), from the host settings query (`useHostSettings`) the
 * settings page updates when they are saved. Never waits: until it loads, or when it fails, a Claude conversation.
 */
export function useAgentDefaults(): AgentDefaults {
  const q = useHostSettings();
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
