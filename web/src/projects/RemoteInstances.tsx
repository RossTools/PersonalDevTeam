import { useQueryClient } from '@tanstack/react-query';
import { useState } from 'react';
import { Plus, RefreshCw, Trash2, X } from 'lucide-react';
import { Alert, Badge, Button, EmptyState, IconButton, Input, Section, Select, SettingsRow, Skeleton, useConfirm, type BadgeTone } from '@ui';
import { api } from '../api/client';
import type { LinkState } from '../api/gen/LinkState';
import type { Project } from '../api/gen/Project';
import type { RemoteInstanceView } from '../api/gen/RemoteInstanceView';
import { AutoSaveText } from '../common/AutoSave';
import { LoadError } from '../common/QueryStatus';
import { formatBytes } from '../common/text';
import { useAction } from '../common/useAction';
import type { PlacementSettings } from '../api/gen/PlacementSettings';
import { projectKeys, usePlacement, useRemoteInstances } from './queries';

export const LINK_STATE: Record<LinkState, { label: string; tone: BadgeTone }> = {
  connected: { label: 'connected', tone: 'running' },
  paused: { label: 'paused', tone: 'waiting' },
  offline: { label: 'offline', tone: 'neutral' },
  incompatible: { label: 'needs an update', tone: 'failed' },
};

const OS: Record<string, string> = { macos: 'macOS', windows: 'Windows', linux: 'Linux' };

/** A cap as typed: blank (or 0) for none. */
export function capOf(text: string): number {
  const n = Number(text.trim() || '0');
  if (!Number.isInteger(n) || n < 0) throw new Error('Enter a whole number, or leave it blank.');
  return n;
}

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
            {f.agentsCheckedAt == null && f.agents.length === 0
              ? 'checking agents…'
              : agents.length === 0 ? 'no agents' : agents.map((a) => `${a.agent}${a.loggedIn ? '' : ' (not logged in)'}`).join(', ')}
          </span>
        )}
        {!!f?.tools.length && <span>{f.tools.map((t) => t.name).join(', ')}</span>}
        <span>{i.lastSeenAt ? `last seen ${ago(i.lastSeenAt)}` : 'not heard from yet'}</span>
        {i.lastError && i.state === 'offline' && <span>{i.lastError}</span>}
      </span>
      <span className="flex flex-wrap items-center gap-2 text-meta text-muted-foreground">
        At most
        <AutoSaveText
          aria-label={`Cap for ${i.name}`}
          inputMode="numeric"
          size="sm"
          className="w-14"
          placeholder={String(i.poolCap)}
          value={i.cap == null ? '' : String(i.cap)}
          onSave={async (t) => {
            qc.setQueryData(projectKeys.remoteInstances(project.id), await api.patchRemoteInstance(project.id, i.linkId, { cap: capOf(t) }));
          }}
        />
        tickets in flight on it at once{i.remoteCap != null && ` (${i.name}'s own cap: ${i.remoteCap})`}. Now {i.poolCap}.
      </span>
    </li>
  );
}

/**
 * Placement: tickets with a label run on an instance (`windows` → win-box). Others run here, unless the repository's
 * `[[placement]]` needs pick an instance whose facts match, or Start's Runs on picks one.
 */
function PlacementRules({ project, instances }: { project: Project; instances: RemoteInstanceView[] }) {
  const qc = useQueryClient();
  const q = usePlacement(project.id);
  const [label, setLabel] = useState('');
  const [instance, setInstance] = useState(instances[0]?.instanceId ?? '');
  const rules = q.data?.rules ?? [];
  const [save, saving] = useAction(async (next: PlacementSettings) => {
    qc.setQueryData(projectKeys.placement(project.id), await api.putPlacement(project.id, next));
  });
  const nameOf = (id: string) => instances.find((i) => i.instanceId === id)?.name ?? 'an instance that left';
  const picked = instances.some((i) => i.instanceId === instance) ? instance : instances[0]?.instanceId ?? '';
  const add = () => {
    if (!label.trim() || !picked) return;
    void save({ rules: [...rules, { label: label.trim(), instance: picked }] }).then(() => setLabel(''));
  };
  return (
    <SettingsRow
      stacked
      label="Placement"
      description="Tickets with one of these labels run on that instance: its worktree, agents and git are there. Other tickets run here. Start's Runs on can pick another."
    >
      <LoadError what="Placement rules" error={q.error} />
      {rules.length > 0 && (
        <ul className="flex flex-col gap-1">
          {rules.map((r) => (
            <li key={r.label} className="flex items-center gap-2 text-sm">
              <span className="font-mono">{r.label}</span>
              <span className="text-muted-foreground">→ {nameOf(r.instance)}</span>
              <IconButton
                size="sm"
                label={`Remove the rule for ${r.label}`}
                disabled={saving}
                onClick={() => void save({ rules: rules.filter((x) => x.label !== r.label) })}
              >
                <X />
              </IconButton>
            </li>
          ))}
        </ul>
      )}
      <div className="flex flex-wrap items-center gap-2">
        <Input mono value={label} placeholder="windows" maxLength={64} aria-label="Label" className="w-40" onChange={(e) => setLabel(e.target.value)} />
        <Select<string>
          aria-label="Instance"
          options={instances.map((i) => ({ value: i.instanceId, label: i.name }))}
          value={picked}
          onChange={setInstance}
          className="w-48"
        />
        <Button size="sm" pending={saving} disabled={!label.trim()} onClick={add}><Plus aria-hidden /> Add rule</Button>
      </div>
    </SettingsRow>
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
      {v && v.instances.length > 0 && <PlacementRules project={project} instances={v.instances} />}
      {v?.pairing ? (
        <SettingsRow
          stacked
          label="Pairing code"
          description={
            <>
              On the other instance, choose Add project → Join a remote, and enter this instance&apos;s address and the
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
