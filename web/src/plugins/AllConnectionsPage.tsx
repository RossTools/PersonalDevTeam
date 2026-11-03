// Settings → All connections (PDT-228): every connection from every plugin in one searchable table (and GitHub's
// login). A row opens its plugin's page, where connections are added and edited.
import { useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { Badge, Input, Page, Skeleton } from '@ui';
import { api } from '../api/client';
import { useConnections } from '../connections/queries';
import { connectionDetails } from '../connections/ConnectionRow';
import { useConnectionKinds } from '../connections/kinds';
import { pluginOf } from './model';
import { usePlugins } from './queries';
import { LoadError } from '../common/QueryStatus';

interface Line {
  id: string;
  name: string;
  plugin: string | null;
  pluginLabel: string;
  details: string;
  failing: boolean;
}

export function AllConnectionsPage({ onOpen }: { onOpen: (pluginId: string) => void }) {
  const plugins = usePlugins();
  const { kindOf } = useConnectionKinds();
  const connections = useConnections();
  const github = useQuery({ queryKey: ['forge-login', 'github'], queryFn: () => api.forgeLogin('github') });
  const [q, setQ] = useState('');
  const ps = plugins.data ?? [];
  const lines: Line[] = [
    ...(github.data?.accounts ?? []).map((a) => ({
      id: `github-login:${a.host}`, name: `GitHub login (${a.host})`, plugin: 'github', pluginLabel: 'GitHub', details: a.login, failing: a.rejectedAt != null,
    })),
    ...(connections.data ?? []).map((c) => {
      const p = pluginOf(ps, c);
      return { id: c.id, name: c.name, plugin: p?.id ?? null, pluginLabel: p?.label ?? c.kind, details: connectionDetails(c, kindOf(c.kind)), failing: c.lastError !== null };
    }),
  ];
  const needle = q.trim().toLowerCase();
  const shown = needle ? lines.filter((l) => `${l.name} ${l.pluginLabel} ${l.details}`.toLowerCase().includes(needle)) : lines;
  return (
    <Page title="All connections" className="max-w-5xl" description="Every connection, from every plugin. Add and edit them on their plugin’s page. Their secrets stay in this machine’s keychain.">
      <LoadError what="Connections" error={connections.error} />
      <LoadError what="Plugins" error={plugins.error} />
      {(connections.isLoading || plugins.isLoading) && <Skeleton label="Loading connections" className="h-24" />}
      {connections.data && plugins.data && (
        <div className="flex flex-col gap-3">
          <Input aria-label="Search connections" placeholder="Search connections" value={q} onChange={(e) => setQ(e.target.value)} />
          {shown.length === 0 ? (
            <p className="text-sm text-muted-foreground">{lines.length ? 'No connections match.' : 'None yet. Add one on its plugin’s page.'}</p>
          ) : (
            <div className="overflow-x-auto rounded-lg border border-border bg-card">
              <table className="w-full text-sm">
                <thead>
                  <tr className="text-left text-xs text-muted-foreground">
                    <th className="px-3 py-2 font-medium">Name</th>
                    <th className="px-3 py-2 font-medium">Plugin</th>
                    <th className="px-3 py-2 font-medium max-sm:hidden">Details</th>
                    <th className="px-3 py-2 font-medium">Status</th>
                  </tr>
                </thead>
                <tbody>
                  {shown.map((l) => (
                    <tr
                      key={l.id}
                      className={l.plugin ? 'cursor-pointer border-t border-border hover:bg-muted' : 'border-t border-border'}
                      onClick={l.plugin ? () => onOpen(l.plugin!) : undefined}
                    >
                      <td className="px-3 py-2 font-medium">
                        {l.plugin
                          ? <button type="button" className="text-left outline-none focus-visible:underline" onClick={(e) => { e.stopPropagation(); onOpen(l.plugin!); }}>{l.name}</button>
                          : l.name}
                      </td>
                      <td className="px-3 py-2">{l.pluginLabel}</td>
                      <td className="max-w-64 truncate px-3 py-2 text-xs text-muted-foreground max-sm:hidden">{l.details}</td>
                      <td className="px-3 py-2">{l.failing ? <Badge tone="failed">Failing</Badge> : <Badge tone="running">OK</Badge>}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </div>
      )}
    </Page>
  );
}
