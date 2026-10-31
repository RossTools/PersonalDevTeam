// One connection on its plugin's page (PDT-228): its name and details, Edit and Delete, why it is failing, and an MCP
// server's tools. Secrets stay in the OS keychain.
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Badge, Button, relativeTime } from '@ui';
import { api } from '../api/client';
import type { Connection } from '../api/gen/Connection';
import { hookUrl, useConnectionKinds } from './kinds';

/** An MCP server's tools: how many (their names on hover), or why listing them failed; List again asks it now. */
export function McpToolsLine({ c }: { c: Connection }) {
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

const DM_STATE: Record<string, string> = {
  connected: 'Connected',
  connecting: 'Connecting…',
  retrying: 'Reconnecting',
  failed: 'Not connected',
  off: 'Off',
};

/** A Discord bot's DMs with the PM (PDT-304): whether its Gateway connection is up, or why not. */
export function DiscordDmLine({ c }: { c: Connection }) {
  const dm = useQuery({ queryKey: ['discord-dm'], queryFn: () => api.discordDm(), refetchInterval: 5_000 });
  const d = dm.data;
  const mine = d?.connection === c.name;
  const state = mine ? d.state : 'off';
  const why = mine ? d.error : null;
  return (
    <p role="status" className="w-full pl-2 text-xs text-muted-foreground">
      DMs with the PM: <span className={state === 'failed' ? 'text-destructive' : state === 'connected' ? 'text-foreground' : undefined}>
        {dm.isError ? 'unknown' : DM_STATE[state] ?? state}
      </span>
      {why && <span className={state === 'failed' ? 'text-destructive' : undefined}>: {why}</span>}
      {!mine && d?.connection && <span> (another bot, {d.connection}, holds them)</span>}
    </p>
  );
}

/** What a connection is, in a line: a webhook's address, a channel, a server, or its URL. */
export function connectionDetails(c: Connection): string {
  if (c.kind === 'webhook') return hookUrl(c.name);
  if (c.kind === 'discord_webhook' && typeof c.config.channel === 'string') return `#${c.config.channel}`;
  if (c.kind === 'discord_bot' && typeof c.config.guildId === 'string') return `Server ${c.config.guildId}`;
  return c.url;
}

export function ConnectionRow({ c, onEdit, onDelete }: { c: Connection; onEdit: () => void; onDelete: () => void }) {
  const { kindOf } = useConnectionKinds();
  const details = connectionDetails(c);
  return (
    <div className="flex flex-wrap items-center gap-2 py-1">
      <span className="min-w-0 truncate text-sm font-medium">{c.name}</span>
      {!c.hasSecret && c.kind !== 'http' && !kindOf(c.kind)?.secretOptional && <Badge tone="waiting">No secret</Badge>}
      {details && (c.kind === 'webhook'
        ? <code className="min-w-0 truncate text-xs text-muted-foreground">{details}</code>
        : <span className="min-w-0 truncate text-xs text-muted-foreground">{details}</span>)}
      {(c.kind === 'discord_bot' || c.kind === 'scryer') && c.config.pmActs === true && <Badge>PM acts without asking</Badge>}
      <span className="flex-1" />
      {c.lastError ? <Badge tone="failed">Failing</Badge> : <Badge tone="running">OK</Badge>}
      <Button size="sm" variant="secondary" onClick={onEdit}>Edit</Button>
      <Button size="sm" variant="secondary" onClick={onDelete}>Delete</Button>
      {c.lastError && (
        <p role="status" className="w-full pl-2 text-xs text-destructive">
          Failing{c.lastErrorAt !== null ? ` since ${relativeTime(c.lastErrorAt)}` : ''}: {c.lastError}
        </p>
      )}
      {c.kind === 'mcp' && <McpToolsLine c={c} />}
      {c.kind === 'discord_bot' && typeof c.config.dmUserId === 'string' && c.config.dmUserId !== '' && <DiscordDmLine c={c} />}
    </div>
  );
}
