// The runs view (Phase 7d spec §4): every run of the workflow with its trigger and status; a run shows its path on the
// graph of the version it ran, its steps, and Retry, Resume (an agent step that timed out or stopped), Cancel and a
// waiting gate's choices.
import { useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Alert, Badge, Button, EmptyState, RelativeTime, Skeleton } from '@ui';
import { cn } from '@ui/lib/utils';
import { api } from '../api/client';
import type { WorkflowRun } from '../api/gen/WorkflowRun';
import type { WorkflowRunDetail } from '../api/gen/WorkflowRunDetail';
import type { WorkflowStep } from '../api/gen/WorkflowStep';
import { GraphView } from './GraphView';
import type { Catalog } from './model';
import { RUN_STATUS, duration, pathOf, runSteps } from './runs';
import { StepList, type StepRow } from './StepList';

const STEP_STATUS: Record<WorkflowStep['status'], StepRow['status']> = {
  pending: { tone: 'neutral', text: 'Pending' },
  running: { tone: 'running', text: 'Running' },
  waiting: { tone: 'waiting', text: 'Waiting' },
  succeeded: { tone: 'accent', text: 'Done' },
  failed: { tone: 'failed', text: 'Failed' },
  skipped: { tone: 'neutral', text: 'Skipped' },
};

function triggerOf(r: WorkflowRun): string {
  const t = r.trigger as { type?: string; ticketKey?: string } | null;
  return [t?.type ?? r.triggerNode, t?.ticketKey].filter(Boolean).join(' · ');
}

interface Answer { question: string; choices: string[] }

function waitingGate(s: WorkflowStep): Answer | null {
  const w = s.wait as { answer?: Answer } | null;
  return s.status === 'waiting' && w?.answer ? w.answer : null;
}

function RunDetail({ id, workflowId, catalog }: { id: string; workflowId: string; catalog: Catalog }) {
  const qc = useQueryClient();
  const run = useQuery({
    queryKey: ['workflow-run', id],
    queryFn: () => api.getWorkflowRun(id),
    refetchInterval: (q) => (q.state.data && ['running', 'waiting'].includes(q.state.data.status) ? 2000 : false),
  });
  const version = run.data?.version;
  const doc = useQuery({
    queryKey: ['workflow-version', workflowId, version],
    queryFn: () => api.workflowVersion(workflowId, version!),
    enabled: version !== undefined,
    staleTime: Infinity,
  });
  const done = (d: WorkflowRunDetail) => {
    qc.setQueryData(['workflow-run', id], d);
    void qc.invalidateQueries({ queryKey: ['workflow-runs'] });
  };
  const retry = useMutation({ mutationFn: () => api.retryWorkflowRun(id), onSuccess: done });
  const resume = useMutation({ mutationFn: () => api.resumeWorkflowRun(id), onSuccess: done });
  const cancel = useMutation({ mutationFn: () => api.cancelWorkflowRun(id), onSuccess: done });
  const answer = useMutation({ mutationFn: (a: { step: string; choice: string }) => api.answerWorkflowStep(id, a.step, a.choice), onSuccess: done });
  const [selected, setSelected] = useState<string | null>(null);
  if (run.error) return <Alert tone="error">The run could not be loaded: {run.error.message}</Alert>;
  if (!run.data) return <Skeleton lines={6} className="p-3" />;
  const r = run.data;
  const status = RUN_STATUS[r.status];
  const d = doc.data?.doc;
  const error = retry.error ?? resume.error ?? cancel.error ?? answer.error;
  const resumeNode = r.steps.find((s) => s.id === r.resumeFrom)?.nodeId;
  return (
    <section aria-label="Run" className="flex min-h-0 flex-1 flex-col gap-3 overflow-auto p-3">
      <div className="flex flex-wrap items-center gap-2 text-sm">
        <Badge tone={status.tone}>{status.text}</Badge>
        <span className="font-medium">{triggerOf(r)}</span>
        <span className="text-meta text-muted-foreground">
          version {r.version} · started <RelativeTime time={r.startedAt} />{r.endedAt ? ` · took ${duration(r.endedAt - r.startedAt)}` : ''}
        </span>
        <span className="ml-auto flex gap-1">
          {resumeNode && (
            <Button size="sm" variant="primary" onClick={() => resume.mutate()} pending={resume.isPending}
              title="Run this agent step again in the same run: same ticket, attempt and session">Resume from {resumeNode}</Button>
          )}
          {r.status === 'failed' && <Button size="sm" variant="primary" onClick={() => retry.mutate()} pending={retry.isPending}>Retry from the failed step</Button>}
          {(r.status === 'running' || r.status === 'waiting') && (
            <Button size="sm" variant="secondary" onClick={() => cancel.mutate()} pending={cancel.isPending}>Cancel</Button>
          )}
        </span>
      </div>
      {r.error && <Alert tone="error">{r.error}</Alert>}
      {error && <Alert tone="error">{error.message}</Alert>}
      {d && (
        <div className="flex h-72 shrink-0 flex-col rounded-md border border-border">
          <GraphView aria-label="Run path" doc={d} catalog={catalog} selected={selected} onSelect={setSelected} readOnly path={pathOf(d, runSteps(r.steps))} />
        </div>
      )}
      {d && (
        <StepList doc={d} catalog={catalog} steps={[...r.steps].sort((a, b) => a.seq - b.seq).map((s) => {
          const gate = waitingGate(s);
          return {
            key: s.id, nodeId: s.nodeId, kind: s.kind, status: STEP_STATUS[s.status], port: s.port, error: s.error,
            input: s.input, output: s.output, attempts: s.attempts,
            ...(gate ? {
              actions: (
                <div className="flex flex-wrap items-center gap-2 rounded-md bg-waiting-soft p-2 text-sm">
                  <span className="min-w-0 flex-1">{gate.question}</span>
                  {gate.choices.map((c) => (
                    <Button key={c} size="sm" variant="secondary" pending={answer.isPending && answer.variables?.choice === c}
                      onClick={() => answer.mutate({ step: s.id, choice: c })}>{c}</Button>
                  ))}
                </div>
              ),
            } : {}),
          };
        })} />
      )}
    </section>
  );
}

export function RunsView({ workflowId, catalog, runId, onRunChange }: {
  workflowId: string; catalog: Catalog; runId: string | null; onRunChange: (id: string | null) => void;
}) {
  const runs = useQuery({ queryKey: ['workflow-runs', workflowId], queryFn: () => api.listWorkflowRuns({ workflow: workflowId, limit: 100 }), refetchInterval: 2000 });
  const list = runs.data ?? [];
  const current = runId ?? list[0]?.id ?? null;
  return (
    <div className="flex min-h-0 flex-1 flex-col md:flex-row">
      <nav aria-label="Runs" className="max-h-56 shrink-0 overflow-auto border-b border-border md:max-h-none md:w-72 md:border-r md:border-b-0">
        {runs.error && <Alert tone="error" className="m-2">Runs could not be loaded: {runs.error.message}</Alert>}
        {runs.isSuccess && list.length === 0 && <EmptyState>No runs yet. Dry run it, or enable it and wait for its trigger.</EmptyState>}
        <ul className="flex flex-col p-1">
          {list.map((r) => {
            const s = RUN_STATUS[r.status];
            return (
              <li key={r.id}>
                <button type="button" aria-current={r.id === current ? 'true' : undefined} onClick={() => onRunChange(r.id)}
                  className={cn('flex w-full flex-col gap-0.5 rounded-md px-2 py-1.5 text-left hover:bg-accent pointer-coarse:min-h-control-touch', r.id === current && 'bg-accent')}>
                  <span className="flex items-center gap-2 text-sm">
                    <Badge tone={s.tone}>{s.text}</Badge>
                    <span className="min-w-0 truncate">{triggerOf(r)}</span>
                  </span>
                  <span className="text-meta text-muted-foreground"><RelativeTime time={r.startedAt} /> · v{r.version}</span>
                </button>
              </li>
            );
          })}
        </ul>
      </nav>
      {current ? <RunDetail key={current} id={current} workflowId={workflowId} catalog={catalog} /> : <div className="flex-1" />}
    </div>
  );
}
