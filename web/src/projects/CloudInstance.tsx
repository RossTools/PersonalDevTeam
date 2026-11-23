import { useQueryClient } from '@tanstack/react-query';
import { useState } from 'react';
import { Cloud, Play, Square } from 'lucide-react';
import { Alert, Badge, Button, Dialog, DialogActions, Field, Input, Select, type BadgeTone } from '@ui';
import { api } from '../api/client';
import type { CloudProviderView } from '../api/gen/CloudProviderView';
import type { CloudUsage } from '../api/gen/CloudUsage';
import type { CloudView } from '../api/gen/CloudView';
import type { Power } from '../api/gen/Power';
import type { Project } from '../api/gen/Project';
import type { RemoteInstanceView } from '../api/gen/RemoteInstanceView';
import { useConnections } from '../connections/queries';
import { useAction } from '../common/useAction';
import { projectKeys } from './queries';

const POWER: Record<Power, { label: string; tone: BadgeTone }> = {
  running: { label: 'VM running', tone: 'running' },
  starting: { label: 'VM starting', tone: 'waiting' },
  stopping: { label: 'VM stopping', tone: 'waiting' },
  stopped: { label: 'VM stopped', tone: 'neutral' },
  unknown: { label: 'VM state unknown', tone: 'neutral' },
};

/** "12.5 h · $4.70 this month" from a meter's newest month, when it is this month. */
export function usageText(usage: CloudUsage[], now = new Date()): string {
  const month = `${now.getUTCFullYear()}-${String(now.getUTCMonth() + 1).padStart(2, '0')}`;
  const u = usage.find((x) => x.month === month);
  return `${(u?.hours ?? 0).toFixed(1)} h · $${(u?.cost ?? 0).toFixed(2)} this month`;
}

/** Cloud VM… : which provider connection and machine an instance is, when it stops, and what it costs an hour. */
function CloudSetupDialog({ project, i, providers, onClose }: { project: Project; i: RemoteInstanceView; providers: CloudProviderView[]; onClose: () => void }) {
  const qc = useQueryClient();
  const connections = (useConnections().data ?? []).filter((c) => providers.some((p) => p.kind === c.kind));
  const now = i.cloud?.setup;
  const [picked, setConnection] = useState(now?.connection ?? '');
  // The first one until the user picks (the list may still be loading on the first render).
  const connection = picked || connections[0]?.name || '';
  const [machine, setMachine] = useState(now?.machine ?? '');
  const [idle, setIdle] = useState(String(now?.idleMinutes ?? 30));
  const [rate, setRate] = useState(now ? String(now.hourlyRate) : '');
  const [error, setError] = useState<string | null>(null);
  const kind = connections.find((c) => c.name === connection)?.kind;
  const hint = providers.find((p) => p.kind === kind)?.machineHint;
  const done = (v: Awaited<ReturnType<typeof api.deleteCloudInstance>>) => {
    qc.setQueryData(projectKeys.remoteInstances(project.id), v);
    onClose();
  };
  const [save, saving] = useAction(async () => {
    const idleMinutes = Number(idle.trim() || '0');
    const hourlyRate = Number(rate.trim() || '0');
    if (!Number.isInteger(idleMinutes) || idleMinutes < 0) return setError('Idle minutes are a whole number (0: never stop it).');
    if (!Number.isFinite(hourlyRate) || hourlyRate < 0) return setError('The hourly rate is dollars an hour (0.376).');
    try {
      done(await api.putCloudInstance(project.id, i.linkId, { connection, machine: machine.trim(), idleMinutes, hourlyRate }));
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  });
  const [remove, removing] = useAction(async () => done(await api.deleteCloudInstance(project.id, i.linkId)));
  return (
    <Dialog
      open
      onOpenChange={(o) => !o && onClose()}
      title={`${i.name} as a cloud VM`}
      description="PDT starts the VM when a ticket is placed on it, and stops it once nothing has run on it for a while."
      footer={(
        <DialogActions
          onCancel={onClose}
          action="Save"
          onAction={() => void save()}
          pending={saving}
          disabled={!connection || !machine.trim()}
          extra={i.cloud && <Button variant="ghost" pending={removing} onClick={() => void remove()}>Not a cloud VM</Button>}
        />
      )}
    >
      <div className="flex flex-col gap-3">
        {connections.length === 0 && <Alert tone="info">Add an AWS or Azure connection first (Connections), with the provider’s credentials.</Alert>}
        <Field label="Connection">
          <Select<string>
            aria-label="Connection"
            options={connections.map((c) => ({ value: c.name, label: c.name }))}
            value={connection || null}
            onChange={setConnection}
          />
        </Field>
        <Field label="Machine" hint={hint && `At this provider, like ${hint}.`}>
          <Input mono value={machine} placeholder={hint} onChange={(e) => setMachine(e.target.value)} />
        </Field>
        <Field label="Stop after idle minutes" hint="No sessions, runs or work waiting for it. 0: never stop it.">
          <Input inputMode="numeric" value={idle} className="w-24" onChange={(e) => setIdle(e.target.value)} />
        </Field>
        <Field label="Hourly rate (US$)" hint="What it costs an hour while it runs, for the cost meter.">
          <Input inputMode="decimal" value={rate} placeholder="0.376" className="w-24" onChange={(e) => setRate(e.target.value)} />
        </Field>
        {error && <Alert tone="error">{error}</Alert>}
      </div>
    </Dialog>
  );
}

/** The VM's state, this month's cost, Start or Stop, and why its last start or check failed. */
function CloudLine({ project, i, cloud }: { project: Project; i: RemoteInstanceView; cloud: CloudView }) {
  const qc = useQueryClient();
  const power = cloud.starting ? { label: 'VM starting', tone: 'waiting' as BadgeTone } : POWER[cloud.state.power ?? 'unknown'];
  const [act, acting] = useAction(async (action: 'start' | 'stop') => {
    qc.setQueryData(projectKeys.remoteInstances(project.id), await api.cloudPower(project.id, i.linkId, action));
  });
  const stopped = !cloud.starting && (cloud.state.power === 'stopped' || cloud.state.power === null);
  return (
    <>
      <span className="flex flex-wrap items-center gap-2 text-meta text-muted-foreground">
        <Badge tone={power.tone}>{power.label}</Badge>
        <span>{usageText(cloud.usage)}</span>
        <span>{cloud.setup.idleMinutes > 0 ? `stops after ${cloud.setup.idleMinutes} idle min` : 'never stopped by PDT'}</span>
        {stopped
          ? <Button size="sm" variant="ghost" pending={acting} onClick={() => void act('start')}><Play aria-hidden /> Start</Button>
          : <Button size="sm" variant="ghost" pending={acting} onClick={() => void act('stop')}><Square aria-hidden /> Stop</Button>}
      </span>
      {cloud.state.lastError && <Alert tone="error">{cloud.state.lastError}</Alert>}
    </>
  );
}

/**
 * A remote instance's cloud side (in Remote instances): for a VM at a cloud provider, its state, cost and Start/Stop;
 * Cloud VM… sets it up (shown once a cloud plugin is on). And, for any instance, which agents need a login there.
 */
export function CloudInstance({ project, i, providers }: { project: Project; i: RemoteInstanceView; providers: CloudProviderView[] }) {
  const [editing, setEditing] = useState(false);
  return (
    <>
      {i.needsLogin.length > 0 && (
        <Alert tone="warning">
          {i.needsLogin.join(', ')} {i.needsLogin.length === 1 ? 'needs' : 'need'} a login on {i.name}: connect to it over RDP or SSH and
          sign in once, or give the project a token secret for remote sessions. Never put credentials in a VM image.
        </Alert>
      )}
      {i.cloud && <CloudLine project={project} i={i} cloud={i.cloud} />}
      {(i.cloud || providers.length > 0) && (
        <span>
          <Button size="sm" variant="ghost" onClick={() => setEditing(true)}><Cloud aria-hidden /> Cloud VM…</Button>
        </span>
      )}
      {editing && <CloudSetupDialog project={project} i={i} providers={providers} onClose={() => setEditing(false)} />}
    </>
  );
}
