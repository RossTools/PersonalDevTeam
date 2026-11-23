import { useQueryClient } from '@tanstack/react-query';
import { ExternalLink, Pause, Play, Trash2 } from 'lucide-react';
import { Badge, Button, EmptyState, Page, Section, Skeleton, useConfirm } from '@ui';
import { api } from '../api/client';
import type { RemoteProjectView } from '../api/gen/RemoteProjectView';
import { openExternal } from '../common/open';
import { LoadError } from '../common/QueryStatus';
import { useAction } from '../common/useAction';
import { JoinRemoteProjectForm } from './JoinRemoteProjectForm';
import { projectKeys, useRemoteProjects } from './queries';
import { LINK_STATE, ago } from './RemoteInstances';

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
    await api.leaveRemoteProject(r.project.id);
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
      <span className="flex flex-wrap gap-x-3 text-meta text-muted-foreground">
        <span className="font-mono">{r.link.ownerUrl}</span>
        <span>{owner} v{r.link.ownerVersion}</span>
        <span>{r.link.lastContactAt ? `last heard from ${ago(r.link.lastContactAt)}` : `waiting for ${owner}`}</span>
        <span>Nothing runs here yet.</span>
      </span>
    </li>
  );
}

/**
 * Settings → Remote projects: the projects other instances own that this one joined to work for them. Their board,
 * tickets and PM are on the owner; here they show their link's state, Pause and Leave.
 */
export function RemoteProjectsPage() {
  const q = useRemoteProjects();
  return (
    <Page title="Remote projects" description="Projects another instance owns, which this one works on for it. Their board, tickets and PM are on the owner.">
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
