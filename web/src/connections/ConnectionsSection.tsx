// Settings → This host → Connections (Phase 7e): the services workflows use by name (a Discord channel's webhook, an
// incoming webhook's secret, an HTTP service and its token).
import { useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Alert, Badge, Button, Section, useConfirm } from '@ui';
import { api } from '../api/client';
import type { Connection } from '../api/gen/Connection';
import { ConnectionDialog, connectionsKey } from './ConnectionDialog';
import { hookUrl, kindLabel } from './kinds';

export function ConnectionsSection() {
  const qc = useQueryClient();
  const confirm = useConfirm();
  const list = useQuery({ queryKey: connectionsKey, queryFn: api.listConnections });
  const [editing, setEditing] = useState<Connection | 'new' | null>(null);
  const remove = useMutation({
    mutationFn: (c: Connection) => api.deleteConnection(c.id),
    onSuccess: () => void qc.invalidateQueries({ queryKey: connectionsKey }),
  });
  const ask = async (c: Connection) => {
    const { ok } = await confirm({
      title: `Delete ${c.name}?`,
      body: 'Workflows that use it fail until you add one with the same name. Its secret is removed from the keychain.',
      action: 'Delete connection',
      danger: true,
    });
    if (ok) remove.mutate(c);
  };
  const items = list.data ?? [];
  return (
    <Section
      label="Connections"
      description="Services workflows use by name: a Discord channel to post to, a webhook other machines call, an HTTP service. Secrets stay in this machine's keychain."
      actions={<Button size="sm" onClick={() => setEditing('new')}>New connection</Button>}
    >
      {list.error && <Alert tone="error">Connections could not be loaded: {list.error.message}</Alert>}
      {!list.isLoading && items.length === 0 && (
        <p className="text-sm text-muted-foreground">
          None yet. To post to Discord, add a Discord webhook: in Discord, open the channel’s settings → Integrations → Webhooks → New Webhook, copy its URL, and paste it here.
        </p>
      )}
      {items.map((c) => (
        <div key={c.id} className="flex flex-wrap items-center gap-2 py-1">
          <span className="min-w-0 truncate text-sm font-medium">{c.name}</span>
          <Badge>{kindLabel(c.kind)}</Badge>
          {!c.hasSecret && c.kind !== 'http' && <Badge tone="waiting">No secret</Badge>}
          {c.kind === 'webhook' && <code className="min-w-0 truncate text-xs text-muted-foreground">{hookUrl(c.name)}</code>}
          {c.url && <span className="min-w-0 truncate text-xs text-muted-foreground">{c.url}</span>}
          <span className="flex-1" />
          <Button size="sm" variant="secondary" onClick={() => setEditing(c)}>Edit</Button>
          <Button size="sm" variant="secondary" onClick={() => void ask(c)}>Delete</Button>
        </div>
      ))}
      {remove.error && <Alert tone="error">{remove.error.message}</Alert>}
      {editing && <ConnectionDialog connection={editing === 'new' ? undefined : editing} onClose={() => setEditing(null)} />}
    </Section>
  );
}
