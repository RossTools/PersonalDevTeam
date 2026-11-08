// One connection on its plugin's page (PDT-228): its name and details, Edit and Delete, why it is failing, and an MCP
// server's tools. Secrets stay in the OS keychain.
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Badge, Button, RelativeTime } from '@ui';
import { api } from '../api/client';
import type { Connection } from '../api/gen/Connection';
import { hookUrl, summaryText, useConnectionKinds, type ConnectionKind } from './kinds';
import { POLL } from '../common/poll';
import { useConnectionStatus } from './queries';

/** An MCP server's tools: how many (their names on hover), or why listing them failed; List again asks it now. */
function McpToolsLine({ c }: { c: Connection }) {
  const qc = useQueryClient();
  const key = ['connection-tools', c.id];
  const tools = useQuery({ queryKey: key, queryFn: () => api.connectionTools(c.id), refetchInterval: POLL.slow });
  const relist = useMutation({ meta: { errorShown: true }, mutationFn: () => api.refreshConnectionTools(c.id), onSuccess: (t) => qc.setQueryData(key, t) });
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

/** How a connection is doing beyond OK or Failing, as its plugin tells it (a bot's DMs with the PM). */
function StatusLine({ c }: { c: Connection }) {
  const status = useConnectionStatus(c.id);
  const s = status.data;
  if (status.isError) return <p role="status" className="w-full pl-2 text-xs text-muted-foreground">Status unknown: {status.error.message}</p>;
  if (!s) return null;
  const tone = s.tone === 'failed' ? 'text-destructive' : s.tone === 'ok' ? 'text-foreground' : undefined;
  return (
    <p role="status" className="w-full pl-2 text-xs text-muted-foreground">
      {s.label}: <span className={tone}>{s.state}</span>
      {s.error && <span className={s.tone === 'failed' ? 'text-destructive' : undefined}>: {s.error}</span>}
    </p>
  );
}

/** What a connection is, in a line: a webhook's address, its kind's summary (a channel, a server), or its URL. */
export function connectionDetails(c: Connection, kind: ConnectionKind | undefined): string {
  if (c.kind === 'webhook') return hookUrl(c.name);
  return (kind?.summary ? summaryText(kind.summary, c.config) : null) ?? c.url;
}

export function ConnectionRow({ c, onEdit, onDelete }: { c: Connection; onEdit: () => void; onDelete: () => void }) {
  const { kindOf } = useConnectionKinds();
  const k = kindOf(c.kind);
  const details = connectionDetails(c, k);
  return (
    <div className="flex flex-wrap items-center gap-2 py-1">
      <span className="min-w-0 truncate text-sm font-medium">{c.name}</span>
      {!c.hasSecret && c.kind !== 'http' && !k?.secretOptional && <Badge tone="waiting">No secret</Badge>}
      {details && (c.kind === 'webhook'
        ? <code className="min-w-0 truncate text-xs text-muted-foreground">{details}</code>
        : <span className="min-w-0 truncate text-xs text-muted-foreground">{details}</span>)}
      {k?.pmActs && c.config.pmActs === true && <Badge>PM acts without asking</Badge>}
      <span className="flex-1" />
      {c.lastError ? <Badge tone="failed">Failing</Badge> : <Badge tone="running">OK</Badge>}
      <Button size="sm" variant="secondary" onClick={onEdit}>Edit</Button>
      <Button size="sm" variant="secondary" onClick={onDelete}>Delete</Button>
      {c.lastError && (
        <p role="status" className="w-full pl-2 text-xs text-destructive">
          Failing{c.lastErrorAt !== null && <> since <RelativeTime time={c.lastErrorAt} /></>}: {c.lastError}
        </p>
      )}
      {c.kind === 'mcp' && <McpToolsLine c={c} />}
      {k?.status && <StatusLine c={c} />}
    </div>
  );
}
