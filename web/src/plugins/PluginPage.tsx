// A plugin's page (PDT-228): its status and switch, a banner when it needs the user, its connections (GitHub's is the
// login), what it adds (workflow steps, agent and PM tools, templates, project settings) and how to set it up.
import { useState, type ReactNode } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Alert, Button, Chip, EmptyState, Link, Page, Section, Skeleton, useConfirm } from '@ui';
import { api } from '../api/client';
import type { Connection } from '../api/gen/Connection';
import { ConnectionDialog, connectionsKey } from '../connections/ConnectionDialog';
import { ConnectionRow } from '../connections/ConnectionRow';
import { ForgeLogin } from './ForgeLogin';
import { CATEGORY_LABEL, connectionsOf, pluginsKey, stuckOn, type Plugin } from './model';
import { PluginIcon, PluginStatusBadge, PluginSwitch, usePluginSwitch, usePlugins } from './parts';

function Tags({ items, empty = 'None' }: { items: string[]; empty?: string }) {
  if (!items.length) return <span className="text-xs text-muted-foreground">{empty}</span>;
  return <div className="flex flex-wrap gap-1.5">{items.map((t) => <Chip key={t}>{t}</Chip>)}</div>;
}

function Part({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex min-w-0 flex-col gap-1.5">
      <h4 className="text-xs font-medium uppercase tracking-wide text-muted-foreground">{label}</h4>
      {children}
    </div>
  );
}

/** The plugin's setup, in short steps from what it declares. */
function setupSteps(p: Plugin): string[] {
  if (p.id === 'github') {
    return [
      'Connect GitHub above (sign in with a code), or paste a token.',
      'Projects whose origin is on GitHub then get their pull requests, CI and merges followed.',
      'Use its steps in a workflow, or let agents use its tools instead of the GitHub CLI.',
      'Not using GitHub? Turn it off while no project’s forge is GitHub: PDT then stops using it for pull requests, CI and issues.',
    ];
  }
  return [
    ...(p.core ? [] : [`Turn ${p.label} on.`]),
    ...p.connectionKinds.map((k) => `Add ${p.connectionKinds.length > 1 ? `a ${k.label}` : 'a connection'}: ${k.setup}`),
    ...(p.projectSettings ? ['Link a project to it in the project’s settings (Plugins).'] : []),
    `Use its steps in a workflow${p.pmTools.length ? ', or ask the PM' : ''}.`,
  ];
}

export interface PluginPageProps {
  id: string;
  /** Back to the catalog. */
  onBack: () => void;
  /** A phone: the top bar has Back. */
  phone?: boolean;
}

export function PluginPage({ id, onBack, phone = false }: PluginPageProps) {
  const qc = useQueryClient();
  const confirm = useConfirm();
  const plugins = usePlugins();
  const connections = useQuery({ queryKey: connectionsKey, queryFn: api.listConnections });
  const sw = usePluginSwitch(plugins.data ?? []);
  const [editing, setEditing] = useState<{ connection?: Connection; kind?: string } | null>(null);
  const check = useMutation({ mutationFn: () => api.checkIntegration(id), onSuccess: (next) => {
    qc.setQueryData(pluginsKey, next);
    void qc.invalidateQueries({ queryKey: connectionsKey });
  } });
  const remove = useMutation({
    mutationFn: (c: Connection) => api.deleteConnection(c.id),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: connectionsKey });
      void qc.invalidateQueries({ queryKey: pluginsKey });
    },
  });
  const back = phone ? null : <Link onClick={onBack} className="text-xs font-normal text-muted-foreground">‹ Plugins</Link>;
  if (!plugins.data) {
    return (
      <Page title="Plugin">
        {plugins.error ? <Alert tone="error">Plugins could not be loaded: {plugins.error.message}</Alert> : <Skeleton label="Loading the plugin" className="h-40" />}
      </Page>
    );
  }
  const p = plugins.data.find((x) => x.id === id);
  if (!p) return <EmptyState className="mt-16 border-0" action={<Button size="sm" onClick={onBack}>All plugins</Button>}>No plugin is called {id}.</EmptyState>;

  const mine = connectionsOf(p, connections.data ?? []);
  const ask = async (c: Connection) => {
    const { ok } = await confirm({
      title: `Delete ${c.name}?`,
      body: 'Workflows that use it fail until you add one with the same name. Its secret is removed from the keychain.',
      action: 'Delete connection',
      danger: true,
    });
    if (ok) remove.mutate(c);
  };
  const failing = (connections.data ?? []).find((c) => c.id === p.errorConnection);
  const addFirst = () => setEditing({ kind: p.connectionKinds[0]?.id });

  return (
    <Page
      title={<span className="flex flex-col items-start gap-3">{back}<span className="flex items-center gap-3"><PluginIcon p={p} size="lg" />{p.label}</span></span>}
      description={`${CATEGORY_LABEL[p.category]} · ${p.description || `This PDT was built without ${p.label}.`}`}
      actions={<><PluginStatusBadge p={p} /><PluginSwitch p={p} pending={sw.pending} onChange={(on) => sw.toggle(p, on)} /></>}
    >
      {stuckOn(p) && <Alert tone="info">{stuckOn(p)}</Alert>}
      {p.status === 'off' && p.canTurnOff && (
        <Alert tone="info">{p.label} is off: PDT doesn’t use it for pull requests, CI or issues, and its steps, triggers, tools and built-in workflows are gone until you turn it on. Updates and bug reports still go through GitHub.</Alert>
      )}
      {p.status === 'needs_setup' && (p.forgeKind ? (
        <Alert tone="warning">PDT needs your {p.label} login and has none: connect {p.label} below.</Alert>
      ) : (
        <Alert tone="warning" action={p.connectionKinds.length > 0 && <Button size="sm" variant="primary" onClick={addFirst}>Add a connection</Button>}>
          {p.label} is on but has no connection yet, so its steps and tools can’t do anything.
        </Alert>
      ))}
      {p.status === 'error' && (
        <Alert
          tone="error"
          action={(p.checkable || failing) && (
            <span className="flex gap-2">
              {p.checkable && <Button size="sm" pending={check.isPending} onClick={() => check.mutate()}>Test again</Button>}
              {failing && <Button size="sm" onClick={() => setEditing({ connection: failing })}>Edit connection</Button>}
            </span>
          )}
        >
          {p.error}
        </Alert>
      )}
      {check.error && <Alert tone="error">{check.error.message}</Alert>}
      {p.status === 'not_built' && <Alert tone="warning">This PDT was built without {p.label}. It can’t be turned on here.</Alert>}

      {p.forgeKind && p.built ? <ForgeLogin kind={p.forgeKind} label={p.label} /> : p.connectionKinds.length > 0 && (
        <Section
          label="Connections"
          description={p.enabled ? 'Secrets stay in this machine’s keychain.' : `Turn ${p.label} on to add connections. Those it has are kept while it is off.`}
          actions={p.enabled && p.connectionKinds.map((k) => (
            <Button key={k.id} size="sm" variant="primary" onClick={() => setEditing({ kind: k.id })}>
              {p.connectionKinds.length > 1 ? `Add ${k.label}` : 'Add'}
            </Button>
          ))}
        >
          {connections.isLoading ? <Skeleton label="Loading connections" className="h-8" />
            : mine.length ? mine.map((c) => <ConnectionRow key={c.id} c={c} onEdit={() => setEditing({ connection: c })} onDelete={() => void ask(c)} />)
              : <p className="text-sm text-muted-foreground">None yet.</p>}
        </Section>
      )}
      {connections.error && <Alert tone="error">Connections could not be loaded: {connections.error.message}</Alert>}
      {remove.error && <Alert tone="error">{remove.error.message}</Alert>}

      {p.built && (
        <Section label="What it adds">
          <div className="grid gap-4 sm:grid-cols-2">
            <Part label="Workflow steps"><Tags items={p.nodeKinds.map((k) => k.title)} /></Part>
            <Part label="Agent and PM tools">
              <Tags items={p.tools} />
              {p.pmTools.length > 0 && p.pmTools.length < p.tools.length && <span className="text-xs text-muted-foreground">The PM has {p.pmTools.join(', ')}.</span>}
            </Part>
            <Part label="Templates"><Tags items={[...p.templates, ...p.workflows].map((t) => t.title)} /></Part>
            <Part label="Project settings">
              {p.projectSettings ? (
                <span className="text-xs">
                  {p.projectSettings}
                  {p.projects.length > 0 && <> Linked: {p.projects.map((x, i) => (
                    <span key={x.id}>{i > 0 && ', '}<Link href={`/settings/projects/${encodeURIComponent(x.key)}/plugins`}>{x.name}</Link></span>
                  ))}.</>}
                </span>
              ) : <span className="text-xs text-muted-foreground">None</span>}
            </Part>
          </div>
        </Section>
      )}

      {p.built && (
        <Section label="Setup">
          <div>
            <ol className="list-decimal space-y-1 pl-5 text-sm text-muted-foreground">
              {setupSteps(p).map((s) => <li key={s}>{s}</li>)}
            </ol>
          </div>
        </Section>
      )}
      {sw.error && <Alert tone="error">{sw.error.message}</Alert>}
      {sw.dialog}
      {editing && <ConnectionDialog connection={editing.connection} kind={editing.kind} onClose={() => setEditing(null)} />}
    </Page>
  );
}
