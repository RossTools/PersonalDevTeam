import { useQueryClient } from '@tanstack/react-query';
import { useState } from 'react';
import { Copy, Plus, RefreshCw, Trash2, X } from 'lucide-react';
import { Alert, Badge, Button, clockTime, EmptyState, IconButton, Input, RelativeTime, Section, Select, SettingsRow, Skeleton, toast, useConfirm, type BadgeTone } from '@ui';
import { api } from '../api/client';
import type { LinkState } from '../api/gen/LinkState';
import type { Project } from '../api/gen/Project';
import type { RemoteInstanceView } from '../api/gen/RemoteInstanceView';
import { AutoSaveText } from '../common/AutoSave';
import { copyToClipboard } from '../common/clipboard';
import { LoadError } from '../common/QueryStatus';
import { formatBytes } from '../common/text';
import { useAction } from '../common/useAction';
import type { PlacementSettings } from '../api/gen/PlacementSettings';
import { projectKeys, usePlacement, useRemoteInstances } from './queries';
import { RemoteAccessOff } from './RemoteAccessOff';

export const LINK_STATE: Record<LinkState, { label: string; tone: BadgeTone }> = {
  connected: { label: 'connected', tone: 'running' },
  paused: { label: 'paused', tone: 'waiting' },
  offline: { label: 'offline', tone: 'neutral' },
  incompatible: { label: 'needs an update', tone: 'failed' },
  removed: { label: 'removed by its owner', tone: 'failed' },
};

const OS: Record<string, string> = { macos: 'macOS', windows: 'Windows', linux: 'Linux' };

/** A cap as typed: blank (or 0) for none. */
export function capOf(text: string): number {
  const n = Number(text.trim() || '0');
  if (!Number.isInteger(n) || n < 0) throw new Error('Enter a whole number, or leave it blank.');
  return n;
}

/** "9 min" left until `at` (ms), at least 1 while it lasts. */
export function minutesLeft(at: number, now = Date.now()): string {
  return `${Math.max(1, Math.ceil((at - now) / 60_000))} min`;
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
      body: `${i.name} stops working on ${project.name}, and the keys that link them are revoked. Pairing again needs a new code.`,
      action: 'Remove',
      danger: true,
    });
    if (!ok) return;
    const { told } = await api.removeRemoteInstance(project.id, i.linkId);
    if (!told) toast.info(`${i.name} couldn't be told; leave ${project.name} there too.`, { description: `It shows the project as removed once it can reach this instance.` });
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
        <span>{i.lastSeenAt ? <>last seen <RelativeTime time={i.lastSeenAt} /></> : 'not heard from yet'}</span>
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

/** One line of the pairing block: what it is, the value in mono, and Copy. */
function PairingValue({ label, value }: { label: string; value: string }) {
  return (
    <>
      <dt className="text-meta text-muted-foreground">{label}</dt>
      <dd className="min-w-0 break-all font-mono text-heading tracking-wide">{value}</dd>
      <dd>
        <Button size="sm" variant="ghost" aria-label={`Copy the ${label.toLowerCase()}`} onClick={() => void copyToClipboard(value, `${label} copied`)}>
          <Copy aria-hidden /> Copy
        </Button>
      </dd>
    </>
  );
}

/**
 * Project Settings → Remote instances: the other instances that joined this project to work for it, each with its
 * link's state and what it reported; Add instance shows a pairing code and this instance's address to enter on the
 * other instance. Pairing needs remote access, so while it is off the section says so and links to it instead.
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
  const address = v?.address;
  const pairing = address ? v?.pairing : null;
  return (
    <Section
      label="Remote instances"
      description="Other instances of PDT that work on this project for this one. The board, tickets and PM stay here."
      actions={v && (
        <>
          {v.instances.length > 0 && (
            <Button size="sm" pending={checking} onClick={() => void check()}><RefreshCw aria-hidden /> Check now</Button>
          )}
          {address && <Button size="sm" pending={adding} onClick={() => void add()}><Plus aria-hidden /> {pairing ? 'New code' : 'Add instance'}</Button>}
        </>
      )}
      notice={v && !address && <RemoteAccessOff />}
    >
      {q.isPending && <Skeleton lines={2} label="Loading remote instances" />}
      <LoadError what="Remote instances" error={q.error} />
      {v && v.instances.length === 0 && !pairing && <EmptyState size="inline">No remote instances yet.</EmptyState>}
      {v && v.instances.length > 0 && (
        <ul data-flush className="flex flex-col divide-y divide-border">
          {v.instances.map((i) => <InstanceRow key={i.linkId} project={project} i={i} />)}
        </ul>
      )}
      {v && v.instances.length > 0 && <PlacementRules project={project} instances={v.instances} />}
      {pairing && address && (
        <SettingsRow
          stacked
          label="Pairing code"
          description="On the other instance, choose Add project → Join a remote, and enter this instance's address and the code. It works once."
        >
          <dl aria-label="Pairing" className="grid grid-cols-[auto_minmax(0,1fr)_auto] items-center gap-x-3 gap-y-1">
            <PairingValue label="Code" value={pairing.code} />
            <PairingValue label="Address" value={address} />
          </dl>
          <div className="flex flex-wrap items-center gap-2 text-meta text-muted-foreground">
            <span title={clockTime(pairing.expiresAt)}>Expires in {minutesLeft(pairing.expiresAt)}</span>
            <span className="flex-1" />
            <Button size="sm" pending={cancelling} onClick={() => void cancel()}>Cancel code</Button>
          </div>
        </SettingsRow>
      )}
    </Section>
  );
}
