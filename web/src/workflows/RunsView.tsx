// The runs view: every run of the workflow with its trigger and status; a run shows its path on the
// graph of the version it ran, its steps, and Retry, Resume (an agent step that timed out or stopped), Cancel and a
// waiting gate's choices. View-only (phones, PDT-120): the list is its own screen, and a run is its steps, each
// opening to its input and output, with nothing to act on.
import { useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Alert, Badge, Button, EmptyState, RelativeTime, Skeleton } from '@ui';
import { cn } from '@ui/lib/utils';
import { api } from '../api/client';
import { WatchButton, Watchers } from '../common/Watch';
import { useOpenSession } from '../tickets/actions';
import type { WorkflowRun } from '../api/gen/WorkflowRun';
import type { WorkflowRunDetail } from '../api/gen/WorkflowRunDetail';
import type { WorkflowStep } from '../api/gen/WorkflowStep';
import { GraphView } from './GraphView';
import type { Catalog } from './model';
import { RUN_STATUS, duration, pathOf, runSteps } from './runs';
import { StepList, type StepRow } from './StepList';
import { workflowKeys } from './queries';
import { POLL } from '../common/poll';

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

/** The session a step ran or told (an agent step's, `session.send`'s), from its output. */
function stepSession(s: WorkflowStep): string | null {
  const id = (s.output as { sessionId?: unknown } | null)?.sessionId;
  return typeof id === 'string' ? id : null;
}

function RunDetail({ id, workflowId, catalog, viewOnly }: { id: string; workflowId: string; catalog: Catalog; viewOnly: boolean }) {
  const qc = useQueryClient();
  const run = useQuery({
    queryKey: workflowKeys.run(id),
    queryFn: () => api.getWorkflowRun(id),
    refetchInterval: (q) => (q.state.data && ['running', 'waiting'].includes(q.state.data.status) ? POLL.live : false),
  });
  const version = run.data?.version;
  const doc = useQuery({
    queryKey: workflowKeys.version(workflowId, version ?? 0),
    queryFn: () => api.workflowVersion(workflowId, version!),
    enabled: version !== undefined,
    staleTime: Infinity,
  });
  const done = (d: WorkflowRunDetail) => {
    qc.setQueryData(workflowKeys.run(id), d);
    void qc.invalidateQueries({ queryKey: workflowKeys.allRuns });
  };
  const retry = useMutation({ meta: { errorShown: true }, mutationFn: () => api.retryWorkflowRun(id), onSuccess: done });
  const resume = useMutation({ meta: { errorShown: true }, mutationFn: () => api.resumeWorkflowRun(id), onSuccess: done });
  const cancel = useMutation({ meta: { errorShown: true }, mutationFn: () => api.cancelWorkflowRun(id), onSuccess: done });
  const answer = useMutation({ meta: { errorShown: true }, mutationFn: (a: { step: string; choice: string }) => api.answerWorkflowStep(id, a.step, a.choice), onSuccess: done });
  const openSession = useOpenSession();
  const open = useMutation({ meta: { errorShown: true }, mutationFn: (id: string) => api.getSession(id), onSuccess: (s) => openSession(s) });
  const [selected, setSelected] = useState<string | null>(null);
  if (run.error) return <Alert tone="error">The run could not be loaded: {run.error.message}</Alert>;
  if (!run.data) return <Skeleton lines={6} className="p-3" />;
  const r = run.data;
  const status = RUN_STATUS[r.status];
  const d = doc.data?.doc;
  const error = retry.error ?? resume.error ?? cancel.error ?? answer.error ?? open.error;
  const resumeNode = r.steps.find((s) => s.id === r.resumeFrom)?.nodeId;
  return (
    <section aria-label="Run" className="flex min-h-0 flex-1 flex-col gap-3 overflow-auto p-3">
      <div className="flex flex-wrap items-center gap-2 text-sm">
        <Badge tone={status.tone}>{status.text}</Badge>
        <span className="font-medium">{triggerOf(r)}</span>
        <span className="text-meta text-muted-foreground">
          version {r.version} · started <RelativeTime time={r.startedAt} />{r.endedAt ? ` · took ${duration(r.endedAt - r.startedAt)}` : ''}
        </span>
        <span className={cn('flex gap-1', viewOnly && 'ml-auto')}><WatchButton of={{ run: r.id }} /></span>
        {!viewOnly && <span className="ml-auto flex gap-1">
          {resumeNode && (
            <Button size="sm" variant="primary" onClick={() => resume.mutate()} pending={resume.isPending}
              title="Run this agent step again in the same run: same ticket, attempt and session">Resume from {resumeNode}</Button>
          )}
          {r.status === 'failed' && <Button size="sm" variant="primary" onClick={() => retry.mutate()} pending={retry.isPending}>Retry from the failed step</Button>}
          {(r.status === 'running' || r.status === 'waiting') && (
            <Button size="sm" variant="secondary" onClick={() => cancel.mutate()} pending={cancel.isPending}>Cancel</Button>
          )}
        </span>}
      </div>
      <Watchers of={{ run: r.id }} />
      {r.error && <Alert tone="error">{r.error}</Alert>}
      {error && <Alert tone="error">{error.message}</Alert>}
      {d && !viewOnly && (
        <div className="flex h-72 shrink-0 flex-col rounded-md border border-border">
          <GraphView aria-label="Run path" doc={d} catalog={catalog} selected={selected} onSelect={setSelected} readOnly path={pathOf(d, runSteps(r.steps))} />
        </div>
      )}
      {d && (
        <StepList doc={d} catalog={catalog} expandable={viewOnly} steps={[...r.steps].sort((a, b) => a.seq - b.seq).map((s) => {
          const gate = waitingGate(s);
          const session = stepSession(s);
          // Its session, which the session lists leave out once it ends with no ticket (PDT-263).
          const openButton = session && (
            <Button size="sm" variant="secondary" className="self-start" pending={open.isPending && open.variables === session}
              onClick={() => open.mutate(session)}>Open session</Button>
          );
          return {
            key: s.id, nodeId: s.nodeId, kind: s.kind, status: STEP_STATUS[s.status], port: s.port, error: s.error,
            input: s.input, output: s.output, attempts: s.attempts,
            ...(gate && viewOnly ? {
              actions: <p className="rounded-md bg-waiting-soft p-2 text-sm">Waiting for an answer: {gate.question}</p>,
            } : gate ? {
              actions: (
                <div className="flex flex-wrap items-center gap-2 rounded-md bg-waiting-soft p-2 text-sm">
                  <span className="min-w-0 flex-1">{gate.question}</span>
                  {gate.choices.map((c) => (
                    <Button key={c} size="sm" variant="secondary" pending={answer.isPending && answer.variables?.choice === c}
                      onClick={() => answer.mutate({ step: s.id, choice: c })}>{c}</Button>
                  ))}
                </div>
              ),
            } : openButton ? { actions: openButton } : {}),
          };
        })} />
      )}
    </section>
  );
}

export function RunsView({ workflowId, catalog, runId, onRunChange, viewOnly = false }: {
  workflowId: string; catalog: Catalog; runId: string | null; onRunChange: (id: string | null) => void;
  /** Phones: the list, or the run picked (no graph, nothing to act on). */
  viewOnly?: boolean;
}) {
  const runs = useQuery({ queryKey: workflowKeys.runs(workflowId), queryFn: () => api.listWorkflowRuns({ workflow: workflowId, limit: 100 }), refetchInterval: POLL.live });
  const list = runs.data ?? [];
  const current = viewOnly ? runId : runId ?? list[0]?.id ?? null;
  if (viewOnly && current) return <RunDetail key={current} id={current} workflowId={workflowId} catalog={catalog} viewOnly />;
  return (
    <div className="flex min-h-0 flex-1 flex-col md:flex-row">
      <nav aria-label="Runs" className={cn('shrink-0 overflow-auto border-b border-border md:max-h-none md:w-72 md:border-r md:border-b-0', viewOnly ? 'min-h-0 flex-1' : 'max-h-56')}>
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
      {viewOnly ? null : current ? <RunDetail key={current} id={current} workflowId={workflowId} catalog={catalog} viewOnly={false} /> : <div className="flex-1" />}
    </div>
  );
}
