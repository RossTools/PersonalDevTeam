// What the catalog and a plugin's page share (PDT-228): the icon, the status, the switch, and turning a plugin on,
// which asks for a connection straight away when it needs one and has none.
import { useState, type ReactNode } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Badge, Switch } from '@ui';
import { cn } from '@ui/lib/utils';
import { api } from '../api/client';
import { ConnectionDialog } from '../connections/ConnectionDialog';
import { useConnectionKinds } from '../connections/kinds';
import { STATUS_LABEL, STATUS_TONE, enabledWith, pluginsKey, type Plugin } from './model';

/** Every plugin (`GET /api/integrations`). */
export const usePlugins = () => useQuery({ queryKey: pluginsKey, queryFn: api.listIntegrations });

/** A plugin's letters on its own colour (the brand's, from the plugin: not a theme colour). */
export function PluginIcon({ p, size = 'md' }: { p: Plugin; size?: 'sm' | 'md' | 'lg' }) {
  return (
    <span
      aria-hidden
      // The plugin's colour is data from its declaration, like an avatar's.
      style={{ backgroundColor: p.icon.color }}
      className={cn(
        'grid shrink-0 place-items-center font-bold text-primary-foreground',
        size === 'sm' ? 'size-6 rounded-md text-xs' : size === 'md' ? 'size-8 rounded-lg text-sm' : 'size-12 rounded-xl text-lg',
      )}
    >
      {p.icon.letters}
    </span>
  );
}

export function PluginStatusBadge({ p }: { p: Plugin }) {
  return <Badge tone={STATUS_TONE[p.status]} className={cn(p.status === 'not_built' && 'border border-dashed border-border bg-transparent')}>{STATUS_LABEL[p.status]}</Badge>;
}

/** The switch, or "Built in" for one of PDT's own. */
export function PluginSwitch({ p, pending, onChange }: { p: Plugin; pending: boolean; onChange: (on: boolean) => void }) {
  if (p.core) return <span className="shrink-0 text-xs text-muted-foreground">Built in</span>;
  return (
    // A card opens its page on click: the switch is not part of that.
    <span className="flex shrink-0" onClick={(e) => e.stopPropagation()}>
      <Switch aria-label={`Use ${p.label}`} checked={p.enabled} disabled={!p.built || pending} onChange={onChange} />
    </span>
  );
}

/** Asks for a plugin's first connection once its kinds (which follow what is on) include it. */
function AskConnection({ p, kind, onClose }: { p: Plugin; kind: string; onClose: () => void }) {
  const kinds = useConnectionKinds();
  const k = kinds.kindOf(kind);
  if (!k) return null;
  return (
    <ConnectionDialog
      kind={kind}
      onClose={onClose}
      cancelLabel="Later"
      description={`${p.label} is on. Add a connection so its steps and tools can use it. You can do this later from its page.`}
    />
  );
}

/**
 * Turning plugins on and off. Turning on one that needs a connection and has none ([`needs_setup`] once it is on)
 * opens the add-connection dialog; closing it leaves the plugin in Needs setup.
 */
export function usePluginSwitch(plugins: Plugin[]): { toggle: (p: Plugin, on: boolean) => void; pending: boolean; error: Error | null; dialog: ReactNode } {
  const qc = useQueryClient();
  const [asking, setAsking] = useState<{ p: Plugin; kind: string } | null>(null);
  const set = useMutation({
    mutationFn: ({ enabled }: { enabled: string[]; id: string; on: boolean }) => api.setIntegrations(enabled),
    onSuccess: (next, { id, on }) => {
      qc.setQueryData(pluginsKey, next);
      // Connection kinds, the node catalog, project settings and the personas' tools all follow what is on.
      void qc.invalidateQueries({ predicate: (q) => q.queryKey[0] !== pluginsKey[0] });
      const p = next.find((x) => x.id === id);
      if (on && p?.status === 'needs_setup' && p.connectionKinds.length > 0) setAsking({ p, kind: p.connectionKinds[0].id });
    },
  });
  return {
    toggle: (p, on) => set.mutate({ enabled: enabledWith(plugins, p.id, on), id: p.id, on }),
    pending: set.isPending,
    error: set.error,
    dialog: asking && <AskConnection p={asking.p} kind={asking.kind} onClose={() => setAsking(null)} />,
  };
}
