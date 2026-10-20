// Settings → Workflows (PDT-76): the workflows on this host with their enabled switch and last run. Enabling one is
// consent to what it does; the builder comes in Phase 7d.
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Badge, Section, SettingsRow, Switch, type BadgeTone } from '@ui';
import { api } from '../api/client';
import type { Workflow } from '../api/gen/Workflow';
import type { WorkflowRun } from '../api/gen/WorkflowRun';
import type { WorkflowRunStatus } from '../api/gen/WorkflowRunStatus';

const STATUS: Record<WorkflowRunStatus, { tone: BadgeTone; text: string }> = {
  running: { tone: 'running', text: 'Running' },
  waiting: { tone: 'waiting', text: 'Waiting' },
  succeeded: { tone: 'accent', text: 'Succeeded' },
  quiet: { tone: 'neutral', text: 'Nothing to do' },
  failed: { tone: 'failed', text: 'Failed' },
  cancelled: { tone: 'neutral', text: 'Cancelled' },
};

function when(ms: number) {
  return new Date(ms).toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' });
}

function Row({ w, last }: { w: Workflow; last: WorkflowRun | undefined }) {
  const qc = useQueryClient();
  const toggle = useMutation({
    mutationFn: (enabled: boolean) => api.patchWorkflow(w.id, { enabled }),
    onSuccess: () => void qc.invalidateQueries({ queryKey: ['workflows'] }),
  });
  const status = last ? STATUS[last.status] : null;
  return (
    <SettingsRow
      label={w.name}
      description={
        <>
          {w.description}
          {last && status && (
            <span className="mt-1 flex items-center gap-1.5">
              Last run {when(last.startedAt)} <Badge tone={status.tone}>{status.text}</Badge>
            </span>
          )}
          {toggle.error && <span className="mt-1 block text-destructive">{toggle.error.message}</span>}
        </>
      }
    >
      {w.builtin && <Badge tone="neutral">Built in</Badge>}
      {w.source === 'repo' && <Badge tone="neutral">In the repo</Badge>}
      <Switch aria-label={`Workflow ${w.name}`} checked={w.enabled} onChange={(on) => toggle.mutate(on)} />
    </SettingsRow>
  );
}

export function WorkflowsSection() {
  const workflows = useQuery({ queryKey: ['workflows'], queryFn: () => api.listWorkflows() });
  const runs = useQuery({ queryKey: ['workflow-runs'], queryFn: () => api.listWorkflowRuns({ limit: 200 }) });
  const latest = new Map<string, WorkflowRun>();
  for (const r of runs.data ?? []) if (!latest.has(r.workflowId)) latest.set(r.workflowId, r);
  const list = workflows.data ?? [];
  return (
    <Section
      label="Workflows"
      description="What PDT does by itself when something happens: telling agents about CI and their pull requests, finishing merged tickets. A workflow acts only while it is on."
    >
      {workflows.error && <span className="text-sm text-destructive">Workflows could not be loaded: {workflows.error.message}</span>}
      {list.map((w) => <Row key={w.id} w={w} last={latest.get(w.id)} />)}
      {workflows.isSuccess && list.length === 0 && <span className="text-sm text-muted-foreground">No workflows yet.</span>}
    </Section>
  );
}
