// Versions (Phase 7d spec §6.2): every version with who wrote it, a diff against the current one, Restore, and Accept or
// Discard for a version an agent proposed.
import { useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Alert, Badge, Button, DiffViewer, EmptyState, RelativeTime, Skeleton, useConfirm } from '@ui';
import { cn } from '@ui/lib/utils';
import { api } from '../api/client';
import type { Author } from '../api/gen/Author';
import type { DocDiff } from '../api/gen/DocDiff';
import type { WorkflowDetail } from '../api/gen/WorkflowDetail';
import type { WorkflowVersion } from '../api/gen/WorkflowVersion';

const AUTHOR: Record<Author, string> = { user: 'You', builtin: 'PDT', repo: 'The repo file', agent: 'An agent' };

function Summary({ d }: { d: DocDiff }) {
  const parts = [
    d.nodesAdded.length && `added ${d.nodesAdded.join(', ')}`,
    d.nodesRemoved.length && `removed ${d.nodesRemoved.join(', ')}`,
    d.nodesChanged.length && `changed ${d.nodesChanged.join(', ')}`,
    (d.edgesAdded.length || d.edgesRemoved.length) && `${d.edgesAdded.length} edges added, ${d.edgesRemoved.length} removed`,
    d.metaChanged && 'name, description or settings changed',
  ].filter(Boolean);
  return <p className="text-sm">{parts.length ? parts.join(' · ') : 'Only positions changed.'}</p>;
}

export function VersionsView({ w, dirty, readOnly, onChanged, initial }: {
  w: WorkflowDetail;
  /** The draft has unsaved changes (Restore and Accept replace it). */
  dirty: boolean;
  readOnly?: boolean;
  onChanged: (d: WorkflowDetail) => void;
  initial?: number | null;
}) {
  const qc = useQueryClient();
  const confirm = useConfirm();
  const versions = useQuery({ queryKey: ['workflow-versions', w.id], queryFn: () => api.workflowVersions(w.id) });
  const [picked, setPicked] = useState<number | null>(initial ?? null);
  const list = versions.data ?? [];
  const current = picked ?? w.proposed ?? list[0]?.version ?? null;
  const v: WorkflowVersion | undefined = list.find((x) => x.version === current);
  // The current version against the one before it; any other against the current one.
  const before = list.find((x) => x.version < w.version && x.status === 'saved')?.version;
  const [from, to] = v?.version === w.version ? [before ?? w.version, w.version] : [w.version, v?.version ?? w.version];
  const diff = useQuery({
    queryKey: ['workflow-diff', w.id, from, to],
    queryFn: () => api.workflowDiff(w.id, from, to),
    enabled: !!v && from !== to,
    staleTime: Infinity,
  });
  const done = (d: WorkflowDetail) => {
    qc.setQueryData(['workflow', w.id], d);
    void qc.invalidateQueries({ queryKey: ['workflows'] });
    void qc.invalidateQueries({ queryKey: ['workflow-versions', w.id] });
    onChanged(d);
  };
  const replaceDraft = async (action: string) =>
    !dirty || (await confirm({ title: `${action} and lose your unsaved changes?`, body: 'The editor has changes that are not saved.', action, danger: true })).ok;
  const restore = useMutation({
    mutationFn: async (n: number) => {
      const doc = (await api.workflowVersion(w.id, n)).doc!;
      return api.saveWorkflow(w.id, { doc, note: `Restored v${n}`, propose: false });
    },
    onSuccess: done,
  });
  const accept = useMutation({ mutationFn: (n: number) => api.acceptWorkflowVersion(w.id, n), onSuccess: (d) => { setPicked(null); done(d); } });
  const discard = useMutation({ mutationFn: (n: number) => api.discardWorkflowVersion(w.id, n), onSuccess: (d) => { setPicked(null); done(d); } });
  const error = restore.error ?? accept.error ?? discard.error;
  if (versions.isLoading) return <Skeleton lines={6} className="p-3" />;
  return (
    <div className="flex min-h-0 flex-1 flex-col md:flex-row">
      <nav aria-label="Versions" className="max-h-56 shrink-0 overflow-auto border-b border-border md:max-h-none md:w-72 md:border-r md:border-b-0">
        <ul className="flex flex-col p-1">
          {list.map((x) => (
            <li key={x.version}>
              <button type="button" aria-current={x.version === current ? 'true' : undefined} onClick={() => setPicked(x.version)}
                className={cn('flex w-full flex-col gap-0.5 rounded-md px-2 py-1.5 text-left hover:bg-accent pointer-coarse:min-h-control-touch', x.version === current && 'bg-accent', x.status === 'discarded' && 'text-muted-foreground')}>
                <span className="flex items-center gap-2 text-sm">
                  <span className="font-medium tabular-nums">v{x.version}</span>
                  {x.version === w.version && <Badge tone="accent">Current</Badge>}
                  {x.status === 'proposed' && <Badge tone="waiting">Proposed</Badge>}
                  {x.status === 'discarded' && <Badge tone="neutral">Discarded</Badge>}
                </span>
                <span className="truncate text-meta text-muted-foreground">{AUTHOR[x.author]} · <RelativeTime time={x.createdAt} />{x.note ? ` · ${x.note}` : ''}</span>
              </button>
            </li>
          ))}
        </ul>
      </nav>
      <section aria-label="Version" className="flex min-h-0 flex-1 flex-col gap-3 overflow-auto p-3">
        {!v ? <EmptyState>No versions.</EmptyState> : (
          <>
            <div className="flex flex-wrap items-center gap-2">
              <h3 className="text-sm font-medium">
                v{v.version}{v.version === w.version ? ' (current)' : ''} · {AUTHOR[v.author]}
              </h3>
              <span className="ml-auto flex gap-1">
                {!readOnly && v.status === 'proposed' && (
                  <>
                    <Button size="sm" variant="secondary" pending={discard.isPending} onClick={() => discard.mutate(v.version)}>Discard</Button>
                    <Button size="sm" variant="primary" pending={accept.isPending} onClick={async () => { if (await replaceDraft('Accept')) accept.mutate(v.version); }}>Accept</Button>
                  </>
                )}
                {!readOnly && v.status !== 'proposed' && v.version !== w.version && (
                  <Button size="sm" variant="secondary" pending={restore.isPending}
                    onClick={async () => { if (await replaceDraft('Restore')) restore.mutate(v.version); }}>Restore this version</Button>
                )}
              </span>
            </div>
            {v.status === 'proposed' && (
              <Alert tone="info">An agent proposed this change. Accepting makes it the current version{w.enabled ? ', which runs from then on' : ''}.</Alert>
            )}
            {error && <Alert tone="error">{error.message}</Alert>}
            {from === to ? <p className="text-sm text-muted-foreground">The first version.</p>
              : diff.data ? (
                <>
                  <p className="text-meta text-muted-foreground">v{from} → v{to}</p>
                  <Summary d={diff.data} />
                  {diff.data.unified ? <DiffViewer diff={diff.data.unified} hideHeaders /> : <p className="text-sm text-muted-foreground">No differences.</p>}
                </>
              ) : diff.error ? <Alert tone="error">{diff.error.message}</Alert> : <Skeleton lines={6} />}
          </>
        )}
      </section>
    </div>
  );
}
