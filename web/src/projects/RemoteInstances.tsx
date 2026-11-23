import { useQueryClient } from '@tanstack/react-query';
import { Plus, RefreshCw, Trash2 } from 'lucide-react';
import { Alert, Badge, Button, EmptyState, Section, SettingsRow, Skeleton, useConfirm, type BadgeTone } from '@ui';
import { api } from '../api/client';
import type { LinkState } from '../api/gen/LinkState';
import type { Project } from '../api/gen/Project';
import type { RemoteInstanceView } from '../api/gen/RemoteInstanceView';
import { LoadError } from '../common/QueryStatus';
import { formatBytes } from '../common/text';
import { useAction } from '../common/useAction';
import { projectKeys, useRemoteInstances } from './queries';

export const LINK_STATE: Record<LinkState, { label: string; tone: BadgeTone }> = {
  connected: { label: 'connected', tone: 'running' },
  paused: { label: 'paused', tone: 'waiting' },
  offline: { label: 'offline', tone: 'neutral' },
  incompatible: { label: 'needs an update', tone: 'failed' },
};

const OS: Record<string, string> = { macos: 'macOS', windows: 'Windows', linux: 'Linux' };

/** "3 min ago" for a time in ms. */
export function ago(at: number, now = Date.now()): string {
  const s = Math.max(0, Math.round((now - at) / 1000));
  if (s < 60) return 'just now';
  const m = Math.round(s / 60);
  if (m < 60) return `${m} min ago`;
  const h = Math.round(m / 60);
  if (h < 48) return `${h} h ago`;
  return `${Math.round(h / 24)} days ago`;
}

function InstanceRow({ project, i }: { project: Project; i: RemoteInstanceView }) {
  const qc = useQueryClient();
  const confirm = useConfirm();
  const state = LINK_STATE[i.state];
  const f = i.facts;
  const agents = (f?.agents ?? []).filter((a) => a.installed);
  const [remove, removing] = useAction(async () => {
    const { ok } = await confirm({
      title: `Remove ${i.name}?`,
      body: `${i.name} stops working on ${project.name}, and the keys that link them are revoked at both ends. Pairing again needs a new code.`,
      action: 'Remove',
      danger: true,
    });
    if (!ok) return;
    await api.removeRemoteInstance(project.id, i.linkId);
    await qc.invalidateQueries({ queryKey: projectKeys.remoteInstances(project.id) });
  });
  return (
    <li className="flex flex-col gap-1 px-4 py-2.5">
      <div className="flex flex-wrap items-center gap-2">
        <span className="font-medium">{i.name}</span>
        <Badge tone={state.tone}>{state.label}</Badge>
        <span className="text-meta text-muted-foreground">v{i.version}</span>
        <span className="flex-1" />
        <Button size="sm" pending={removing} onClick={() => void remove()}><Trash2 aria-hidden /> Remove…</Button>
      </div>
      {i.problem && <Alert tone="error">{i.problem}</Alert>}
      <span className="flex flex-wrap gap-x-3 text-meta text-muted-foreground">
        {f && <span>{OS[f.os] ?? f.os} · {f.cpus} CPUs</span>}
        {f?.diskFreeBytes != null && <span>{formatBytes(f.diskFreeBytes)} free</span>}
        {f && (
          <span>
            {agents.length === 0 ? 'no agents' : agents.map((a) => `${a.agent}${a.loggedIn ? '' : ' (not logged in)'}`).join(', ')}
          </span>
        )}
        {!!f?.tools.length && <span>{f.tools.map((t) => t.name).join(', ')}</span>}
        <span>{i.lastSeenAt ? `last seen ${ago(i.lastSeenAt)}` : 'not heard from yet'}</span>
        {i.lastError && i.state === 'offline' && <span>{i.lastError}</span>}
      </span>
    </li>
  );
}

/**
 * Project Settings → Remote instances: the other instances that joined this project to work for it, each with its
 * link's state and what it reported; Add shows a pairing code to enter on the other instance.
 */
export function RemoteInstances({ project }: { project: Project }) {
  const qc = useQueryClient();
  const q = useRemoteInstances(project.id);
  const v = q.data;
  const set = (next: typeof v) => qc.setQueryData(projectKeys.remoteInstances(project.id), next);
  const [add, adding] = useAction(async () => set(await api.newPairingCode(project.id)));
  const [check, checking] = useAction(async () => set(await api.checkRemoteInstances(project.id)));
  const [cancel, cancelling] = useAction(async () => {
    await api.cancelPairingCode(project.id);
    await qc.invalidateQueries({ queryKey: projectKeys.remoteInstances(project.id) });
  });
  return (
    <Section
      label="Remote instances"
      description="Other instances of PDT that work on this project for this one. The board, tickets and PM stay here."
    >
      {q.isPending && <Skeleton lines={2} label="Loading remote instances" />}
      <LoadError what="Remote instances" error={q.error} />
      {v && v.instances.length === 0 && !v.pairing && <EmptyState>No remote instances yet.</EmptyState>}
      {v && v.instances.length > 0 && (
        <ul data-flush className="flex flex-col divide-y divide-border">
          {v.instances.map((i) => <InstanceRow key={i.linkId} project={project} i={i} />)}
        </ul>
      )}
      {v?.pairing ? (
        <SettingsRow
          stacked
          label="Pairing code"
          description={
            <>
              On the other instance, choose Add project → Remote project, and enter this instance&apos;s address and the
              code. It works once, until {new Date(v.pairing.expiresAt).toLocaleTimeString()}.
            </>
          }
        >
          <div className="flex flex-wrap items-center gap-3">
            <code className="rounded border border-border px-2 py-1 font-mono text-heading tracking-widest">{v.pairing.code}</code>
            <span className="text-xs">
              Address: {v.address ? <span className="font-mono">{v.address}</span> : <span className="text-muted-foreground">turn on remote access first (Settings → This host)</span>}
            </span>
            <Button size="sm" variant="ghost" pending={cancelling} onClick={() => void cancel()}>Cancel</Button>
          </div>
        </SettingsRow>
      ) : null}
      {v && (
        <div className="flex flex-wrap gap-2">
          <Button size="sm" pending={adding} onClick={() => void add()}><Plus aria-hidden /> {v.pairing ? 'New code' : 'Add'}</Button>
          {v.instances.length > 0 && (
            <Button size="sm" variant="ghost" pending={checking} onClick={() => void check()}><RefreshCw aria-hidden /> Check now</Button>
          )}
        </div>
      )}
      {v && !v.address && !v.pairing && (
        <p className="text-meta text-muted-foreground">Remote instances reach this one through remote access (Settings → This host → Remote access).</p>
      )}
    </Section>
  );
}
