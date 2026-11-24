// Settings → This host → Updates → Remote instances (PDT-637): the instances joined to this instance's projects, which
// of them need updating (an older version, or a link protocol they can't speak with this one), and Update on each.
// An instance updates on this one's channel unless its remote project there says to use its own.
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { Alert, Badge, Button, Section, toast, useConfirm } from '@ui';
import { capitalize } from '../common/text';
import { api } from '../api/client';
import { ApiError } from '../api/core';
import type { InstanceUpdateReply } from '../api/gen/InstanceUpdateReply';
import type { InstanceUpdateView } from '../api/gen/InstanceUpdateView';
import { POLL } from '../common/poll';
import { useAction } from '../common/useAction';
import { LINK_STATE } from '../projects/RemoteInstances';
import { hostKeys } from './queries';

function InstanceRow({ i }: { i: InstanceUpdateView }) {
  const qc = useQueryClient();
  const confirm = useConfirm();
  const state = LINK_STATE[i.state];
  const [update, updating] = useAction(async () => {
    let reply: InstanceUpdateReply;
    try {
      reply = await api.updateInstance(i.instanceId, false);
    } catch (e) {
      // Agents working there: it asks first, as this instance's own update does.
      if (!(e instanceof ApiError && e.code === 'host_busy')) throw e;
      const { ok } = await confirm({ title: `Update ${i.name} now?`, body: e.message, action: 'Update anyway' });
      if (!ok) return;
      reply = await api.updateInstance(i.instanceId, true);
    }
    await qc.invalidateQueries({ queryKey: hostKeys.instanceUpdates });
    toast.success(reply.outcome === 'installing'
      ? `${reply.name} is installing v${reply.version} (${reply.channel}); it restarts when done.`
      : `${reply.name} is up to date (v${reply.version}, ${reply.channel}).`);
  });
  return (
    <li className="flex flex-col gap-1 py-2">
      <div className="flex flex-wrap items-center gap-2">
        <span className="font-medium">{i.name}</span>
        <span className="text-meta text-muted-foreground">v{i.version} · {i.projects.join(', ')}</span>
        <Badge tone={state.tone}>{state.label}</Badge>
        <span className="flex-1" />
        <Button size="sm" variant={i.needed ? 'primary' : 'secondary'} pending={updating} onClick={() => void update()}>
          Update {i.name}
        </Button>
      </div>
      {i.needed && <Alert tone="warning">{capitalize(i.needed)}.</Alert>}
    </li>
  );
}

/** The joined instances' updates; nothing when no instance has joined one of this instance's projects. */
export function InstanceUpdates() {
  const q = useQuery({ queryKey: hostKeys.instanceUpdates, queryFn: api.instanceUpdates, refetchInterval: POLL.steady });
  const v = q.data;
  if (!v?.instances.length) return null;
  return (
    <Section
      label="Remote instances"
      description={`Instances working on ${v.name}'s projects. Each updates on this host's ${v.channel} channel, unless its remote project there says to use its own.`}
    >
      <ul data-flush className="flex flex-col divide-y divide-border px-4">
        {v.instances.map((i) => <InstanceRow key={i.instanceId} i={i} />)}
      </ul>
    </Section>
  );
}
