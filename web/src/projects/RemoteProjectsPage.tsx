import { useState } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import { ExternalLink, Pause, Play, Trash2 } from 'lucide-react';
import { Alert, Badge, Button, EmptyState, Input, Page, Section, Skeleton, Switch, toast, useConfirm } from '@ui';
import { api } from '../api/client';
import type { RemoteProjectView } from '../api/gen/RemoteProjectView';
import type { Secret } from '../api/gen/Secret';
import { openExternal } from '../common/open';
import { LoadError } from '../common/QueryStatus';
import { useAction } from '../common/useAction';
import { secretKeys, useSecrets } from '../secrets/queries';
import { JoinRemoteProjectForm } from './JoinRemoteProjectForm';
import { projectKeys, useRemoteProjects } from './queries';
import { AutoSaveText } from '../common/AutoSave';
import { LINK_STATE, ago, capOf } from './RemoteInstances';

/** Where this instance keeps the project's checkout and worktrees; empty goes back to the default folder. */
function CheckoutFolder({ r }: { r: RemoteProjectView }) {
  const qc = useQueryClient();
  const [path, setPath] = useState<string | null>(null);
  const [save, saving] = useAction(async () => {
    await api.patchRemoteProject(r.project.id, { checkoutPath: (path ?? '').trim() });
    setPath(null);
    await qc.invalidateQueries({ queryKey: projectKeys.remote });
  });
  return (
    <div className="flex flex-wrap items-center gap-2 text-meta">
      <span className="text-muted-foreground">Checkout</span>
      <Input
        mono
        size="sm"
        aria-label={`Checkout folder of ${r.project.name}`}
        className="min-w-72 flex-1"
        value={path ?? r.checkoutPath}
        maxLength={1024}
        onChange={(e) => setPath(e.target.value)}
      />
      {path !== null && path !== r.checkoutPath && (
        <Button size="sm" pending={saving} onClick={() => void save()}>Save</Button>
      )}
    </div>
  );
}

/**
 * The secrets this instance keeps for the project (a code-signing certificate, a token for a tool signed in here):
 * its owner's agents running here use them, and they win over the owner's of the same name. They stay in this
 * machine's keychain and never cross the link; the owner sees only their names.
 */
function ProjectSecrets({ r }: { r: RemoteProjectView }) {
  const qc = useQueryClient();
  const confirm = useConfirm();
  const all = useSecrets();
  const [name, setName] = useState('');
  const [value, setValue] = useState('');
  const mine = (all.data ?? []).filter((s) => s.projects.includes(r.project.id));
  const refresh = () => qc.invalidateQueries({ queryKey: secretKeys.all });
  const [add, adding] = useAction(async () => {
    await api.createSecret({ name: name.trim(), description: `For ${r.project.name}`, projects: [r.project.id], value });
    setName('');
    setValue('');
    await refresh();
  });
  const [remove, removing] = useAction(async (s: Secret) => {
    const others = s.projects.filter((p) => p !== r.project.id);
    const { ok } = await confirm({
      title: `Remove ${s.name} from ${r.project.name}?`,
      body: others.length
        ? `${r.link.ownerName}'s agents here stop using it; other projects keep it.`
        : `${r.link.ownerName}'s agents here use ${r.link.ownerName}'s own ${s.name}, if it has one. Its value is removed from this machine's keychain.`,
      action: 'Remove',
      danger: true,
    });
    if (!ok) return;
    if (others.length) await api.patchSecret(s.id, { projects: others });
    else await api.deleteSecret(s.id);
    await refresh();
  });
  const ok = /^[A-Za-z][A-Za-z0-9_-]*$/.test(name.trim()) && value.length >= 6;
  return (
    <div className="flex flex-col gap-1 text-meta">
      <span className="text-muted-foreground">
        Secrets kept here for {r.project.name}: {r.link.ownerName}'s agents here use them over its own of the same name, and they never leave this machine.
      </span>
      {mine.length > 0 && (
        <ul aria-label={`Secrets for ${r.project.name}`} className="flex flex-col gap-0.5">
          {mine.map((s) => (
            <li key={s.id} className="flex flex-wrap items-center gap-2">
              <code>{s.name}</code>
              <span className="text-muted-foreground">{s.lastUsedAt !== null ? `used ${ago(s.lastUsedAt)}` : 'not used yet'}</span>
              <Button size="sm" variant="ghost" pending={removing} onClick={() => void remove(s)}>Remove…</Button>
            </li>
          ))}
        </ul>
      )}
      <div className="flex flex-wrap items-center gap-2">
        <Input mono size="sm" aria-label={`New secret's name for ${r.project.name}`} placeholder="NAME" className="w-48" value={name} onChange={(e) => setName(e.target.value)} />
        <Input size="sm" type="password" autoComplete="off" aria-label={`New secret's value for ${r.project.name}`} placeholder="Value" className="min-w-48 flex-1" value={value} onChange={(e) => setValue(e.target.value)} />
        <Button size="sm" disabled={!ok} pending={adding} onClick={() => void add()}>Add secret</Button>
      </div>
    </div>
  );
}

/** The most of the project's tickets this instance takes at once; blank leaves it to the owner. */
function Cap({ r }: { r: RemoteProjectView }) {
  const qc = useQueryClient();
  return (
    <div className="flex flex-wrap items-center gap-2 text-meta text-muted-foreground">
      At most
      <AutoSaveText
        aria-label={`Cap for ${r.project.name}`}
        inputMode="numeric"
        size="sm"
        className="w-14"
        placeholder="—"
        value={r.link.cap == null ? '' : String(r.link.cap)}
        onSave={async (t) => {
          await api.patchRemoteProject(r.project.id, { cap: capOf(t) });
          await qc.invalidateQueries({ queryKey: projectKeys.remote });
        }}
      />
      of its tickets here at once (blank: as many as {r.link.ownerName} sends).
    </div>
  );
}

/** Whose update channel an update the owner asks for uses. */
function OwnUpdateChannel({ r }: { r: RemoteProjectView }) {
  const qc = useQueryClient();
  const [save, saving] = useAction(async (own: boolean) => {
    await api.patchRemoteProject(r.project.id, { ownUpdateChannel: own });
    await qc.invalidateQueries({ queryKey: projectKeys.remote });
  });
  return (
    <Switch
      size="sm"
      checked={r.link.ownUpdateChannel}
      disabled={saving}
      onChange={(own) => void save(own)}
    >
      When {r.link.ownerName} updates this instance, use this host's own update channel (else {r.link.ownerName}'s)
    </Switch>
  );
}

function RemoteProjectRow({ r }: { r: RemoteProjectView }) {
  const qc = useQueryClient();
  const confirm = useConfirm();
  const owner = r.link.ownerName;
  const state = LINK_STATE[r.state];
  const refresh = () => qc.invalidateQueries({ queryKey: projectKeys.remote });
  const [pause, pausing] = useAction(async () => {
    await api.patchRemoteProject(r.project.id, { paused: !r.link.paused });
    await refresh();
  });
  const [remove, removing] = useAction(async () => {
    const { ok } = await confirm({
      title: `Leave ${r.project.name}?`,
      body: `This instance stops working on ${r.project.name} for ${owner}, and the keys that link them are revoked. ${owner} keeps the project; joining again needs a new code.`,
      action: 'Leave project',
      danger: true,
    });
    if (!ok) return;
    const { told } = await api.leaveRemoteProject(r.project.id);
    if (!told) toast.info(`${owner} couldn't be told; remove this instance from ${r.project.name} there too.`);
    await refresh();
  });
  return (
    <li className="flex flex-col gap-1 px-4 py-2.5">
      <div className="flex flex-wrap items-center gap-2">
        <span className="font-medium">{r.project.name}</span>
        <span className="text-muted-foreground">· remote project of {owner}</span>
        <Badge tone={state.tone}>{state.label}</Badge>
        <span className="flex-1" />
        <Button size="sm" variant="ghost" onClick={() => openExternal(r.openUrl)}><ExternalLink aria-hidden /> Open on {owner}</Button>
        <Button size="sm" pending={pausing} onClick={() => void pause()}>
          {r.link.paused ? <><Play aria-hidden /> Resume</> : <><Pause aria-hidden /> Pause</>}
        </Button>
        <Button size="sm" variant="ghost" pending={removing} onClick={() => void remove()}><Trash2 aria-hidden /> Leave…</Button>
      </div>
      {r.state === 'removed' && (
        <Alert tone="warning">{owner} removed this instance from {r.project.name}, or removed the project. Leave it to finish here.</Alert>
      )}
      <span className="flex flex-wrap gap-x-3 text-meta text-muted-foreground">
        <span className="font-mono">{r.link.ownerUrl}</span>
        <span>{owner} v{r.link.ownerVersion}</span>
        <span>{r.link.lastContactAt ? `last heard from ${ago(r.link.lastContactAt)}` : `waiting for ${owner}`}</span>
        {r.running.length === 0 && <span>Nothing runs here for {owner} now.</span>}
      </span>
      {r.running.length > 0 && (
        <ul aria-label={`Running for ${owner}`} className="flex flex-col gap-0.5 text-meta">
          {r.running.map((s) => (
            <li key={s.sessionId} className="flex flex-wrap items-center gap-2">
              <Badge tone={s.live ? 'running' : 'neutral'}>{s.live ? 'running' : 'parked'}</Badge>
              {s.ticketKey && <span className="font-mono">{s.ticketKey}</span>}
              <span className="truncate">{s.title}</span>
              <span className="text-muted-foreground">{s.agent} · since {ago(s.startedAt)}</span>
            </li>
          ))}
        </ul>
      )}
      <CheckoutFolder r={r} />
      <ProjectSecrets r={r} />
      <OwnUpdateChannel r={r} />
      <Cap r={r} />
    </li>
  );
}

/**
 * Settings → Remote projects: the projects other instances own that this one joined to work for them. Their board,
 * tickets and PM are on the owner; here they show their link's state, what runs here for the owner (read-only), the
 * checkout's folder, the secrets kept here for the project, whose update channel the owner's updates use, Pause and
 * Leave.
 */
export function RemoteProjectsPage() {
  const q = useRemoteProjects();
  return (
    <Page crumb="Settings" title="Remote projects" description="Projects another instance owns, which this one works on for it. Their board, tickets and PM are on the owner.">
      <Section label="Joined">
        {q.isPending && <Skeleton lines={2} label="Loading remote projects" />}
        <LoadError what="Remote projects" error={q.error} />
        {q.data?.length === 0 && <EmptyState>No remote projects. Join one below.</EmptyState>}
        {!!q.data?.length && (
          <ul data-flush className="flex flex-col divide-y divide-border">
            {q.data.map((r) => <RemoteProjectRow key={r.project.id} r={r} />)}
          </ul>
        )}
      </Section>
      <Section label="Join a remote project" description="On the owner, open the project's settings → Remote instances → Add for a code.">
        <JoinRemoteProjectForm />
      </Section>
    </Page>
  );
}
