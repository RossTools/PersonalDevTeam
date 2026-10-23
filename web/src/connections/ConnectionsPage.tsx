// Settings → Connections (Phase 7e): the services workflows use by name, a section per kind (a Discord channel's
// webhook, a bot that lets the PM manage a Discord server, an incoming webhook and its secret, an HTTP service and its
// token, Radarr, Sonarr, Plex, MCP servers and their tools). Secrets stay in the OS keychain.
import { useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Alert, Badge, Button, Page, Section, Skeleton, useConfirm } from '@ui';
import { api } from '../api/client';
import type { Connection } from '../api/gen/Connection';
import { ConnectionDialog, connectionsKey } from './ConnectionDialog';
import { CONNECTION_KINDS, hookUrl, kindOf } from './kinds';

/** An MCP server's tools: how many (their names on hover), or why listing them failed; List again asks it now. */
function McpToolsLine({ c }: { c: Connection }) {
  const qc = useQueryClient();
  const key = ['connection-tools', c.id];
  const tools = useQuery({ queryKey: key, queryFn: () => api.connectionTools(c.id), refetchInterval: 10_000 });
  const relist = useMutation({ mutationFn: () => api.refreshConnectionTools(c.id), onSuccess: (t) => qc.setQueryData(key, t) });
  const t = tools.data;
  return (
    <div className="flex w-full flex-wrap items-center gap-2 pl-2 text-xs text-muted-foreground">
      {!t && <span>{tools.isLoading ? 'Loading tools…' : 'Tools not listed yet.'}</span>}
      {t && t.listedAt !== null && (
        <span title={t.tools.map((x) => x.name).join(', ')}>{t.tools.length === 1 ? '1 tool' : `${t.tools.length} tools`}: {t.tools.slice(0, 6).map((x) => x.name).join(', ')}{t.tools.length > 6 ? ', …' : ''}</span>
      )}
      {t?.error && <span className="text-destructive">{t.error}</span>}
      {relist.isError && <span role="alert" className="text-destructive">{relist.error.message}</span>}
      <Button size="sm" variant="ghost" pending={relist.isPending} onClick={() => relist.mutate()}>List again</Button>
    </div>
  );
}

function Row({ c, onEdit, onDelete }: { c: Connection; onEdit: () => void; onDelete: () => void }) {
  return (
    <div className="flex flex-wrap items-center gap-2 py-1">
      <span className="min-w-0 truncate text-sm font-medium">{c.name}</span>
      {!c.hasSecret && c.kind !== 'http' && !kindOf(c.kind)?.secretOptional && <Badge tone="waiting">No secret</Badge>}
      {c.kind === 'webhook' && <code className="min-w-0 truncate text-xs text-muted-foreground">{hookUrl(c.name)}</code>}
      {c.kind === 'discord_webhook' && typeof c.config.channel === 'string' && <span className="text-xs text-muted-foreground">#{c.config.channel}</span>}
      {c.kind === 'discord_bot' && typeof c.config.guildId === 'string' && <span className="text-xs text-muted-foreground">Server {c.config.guildId}</span>}
      {c.kind === 'discord_bot' && c.config.pmActs === true && <Badge>PM acts without asking</Badge>}
      {c.url && c.kind !== 'discord_bot' && <span className="min-w-0 truncate text-xs text-muted-foreground">{c.url}</span>}
      <span className="flex-1" />
      <Button size="sm" variant="secondary" onClick={onEdit}>Edit</Button>
      <Button size="sm" variant="secondary" onClick={onDelete}>Delete</Button>
      {c.kind === 'mcp' && <McpToolsLine c={c} />}
    </div>
  );
}

export function ConnectionsPage() {
  const qc = useQueryClient();
  const confirm = useConfirm();
  const list = useQuery({ queryKey: connectionsKey, queryFn: api.listConnections });
  const [editing, setEditing] = useState<{ connection?: Connection; kind?: string } | null>(null);
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
  const known = new Set(CONNECTION_KINDS.map((k) => k.id));
  const other = items.filter((c) => !known.has(c.kind));
  const rows = (cs: Connection[]) =>
    cs.map((c) => <Row key={c.id} c={c} onEdit={() => setEditing({ connection: c })} onDelete={() => void ask(c)} />);
  return (
    <Page title="Connections" description="Services workflows use by name. Their secrets are kept in this machine's keychain and never shown again.">
      {list.error && <Alert tone="error">Connections could not be loaded: {list.error.message}</Alert>}
      {list.isLoading && <Skeleton label="Loading connections" className="h-24" />}
      {!list.isLoading && CONNECTION_KINDS.map((k) => {
        const mine = items.filter((c) => c.kind === k.id);
        return (
          <Section key={k.id} label={k.plural} description={k.setup}
            actions={<Button size="sm" onClick={() => setEditing({ kind: k.id })}>{`Add ${k.label.toLowerCase()}`}</Button>}>
            {mine.length ? rows(mine) : <p className="text-sm text-muted-foreground">None yet.</p>}
          </Section>
        );
      })}
      {other.length > 0 && <Section label="Other" description="Connections of kinds PDT's actions don't use yet.">{rows(other)}</Section>}
      {remove.error && <Alert tone="error">{remove.error.message}</Alert>}
      {editing && <ConnectionDialog connection={editing.connection} kind={editing.kind} onClose={() => setEditing(null)} />}
    </Page>
  );
}
