import { useMutation, useQueryClient } from '@tanstack/react-query';
import { Switch } from '@ui';
import { api } from '../api/client';
import type { AgentKind } from '../api/gen/AgentKind';
import type { HostSettingsView } from '../api/gen/HostSettingsView';

/** Pure: the agents switched off (none in settings saved before PDT-270 read them). */
export const agentsOff = (v: HostSettingsView): AgentKind[] => v.settings.disabledAgents ?? [];

/** Pure: the agents switched off once `agent` is switched `on` or off. */
export const switched = (off: readonly AgentKind[], agent: AgentKind, on: boolean): AgentKind[] =>
  on ? off.filter((a) => a !== agent) : off.includes(agent) ? [...off] : [...off, agent];

/** Pure: why an agent's switch can't be used (it is the default agent), else undefined. */
export const switchBlocked = (v: HostSettingsView, agent: AgentKind): string | undefined =>
  v.settings.defaultAgent === agent ? 'The default agent: pick another default before switching it off.' : undefined;

/** Pure: what a row says about an agent that is switched off, else undefined. */
export const offNote = (v: HostSettingsView, agent: AgentKind): string | undefined =>
  agentsOff(v).includes(agent) ? 'Switched off: PDT offers it nowhere and starts no new session on it.' : undefined;

/**
 * An agent's on/off switch (Settings → Agents, PDT-270). A switched-off agent leaves every picker and starts no new
 * session; its running sessions carry on. The default agent's switch is disabled.
 */
export function AgentSwitch({ v, agent, label }: { v: HostSettingsView; agent: AgentKind; label: string }) {
  const qc = useQueryClient();
  const save = useMutation({
    meta: { errorShown: true },
    mutationFn: (on: boolean) => api.saveHostSettings({ disabledAgents: switched(agentsOff(v), agent, on) }),
    onSuccess: (view) => {
      qc.setQueryData(['host-settings'], view);
      // Get started counts only the agents switched on.
      void qc.invalidateQueries({ queryKey: ['host-setup'] });
    },
  });
  return (
    <>
      {save.error && <span role="alert" className="text-xs text-destructive">{save.error.message}</span>}
      <Switch
        size="sm"
        aria-label={`Use ${label}`}
        checked={!agentsOff(v).includes(agent)}
        disabled={!!switchBlocked(v, agent) || save.isPending}
        onChange={(on) => save.mutate(on)}
      />
    </>
  );
}
