// A run (or dry run) as the graph and outline draw it: each node's latest status and the edges it took.
import type { FlowStatus } from '@ui/FlowGraph';
import type { DryStep } from '../api/gen/DryStep';
import type { WorkflowDoc } from '../api/gen/WorkflowDoc';
import type { WorkflowStep } from '../api/gen/WorkflowStep';
import type { WorkflowRunStatus } from '../api/gen/WorkflowRunStatus';
import type { BadgeTone } from '@ui';
import { edgeKey, type PathOverlay } from './GraphView';

interface PathStep { nodeId: string; status: FlowStatus; port: string | null }

const STEP: Record<WorkflowStep['status'], FlowStatus> = {
  pending: 'running', running: 'running', waiting: 'waiting', succeeded: 'succeeded', failed: 'failed', skipped: 'skipped',
};

export function runSteps(steps: WorkflowStep[]): PathStep[] {
  return [...steps].sort((a, b) => a.seq - b.seq).map((s) => ({ nodeId: s.nodeId, status: STEP[s.status], port: s.port }));
}

export function dryRunSteps(steps: DryStep[]): PathStep[] {
  return steps.map((s) => ({ nodeId: s.nodeId, status: s.error ? 'failed' : 'succeeded', port: s.port }));
}

/** Each node's latest status, and an edge taken when its node left by its port and its target ran afterwards. */
export function pathOf(doc: WorkflowDoc, steps: PathStep[]): PathOverlay {
  const status = new Map<string, FlowStatus>();
  const taken = new Set<string>();
  steps.forEach((s, i) => {
    status.set(s.nodeId, s.status);
    if (!s.port) return;
    const later = new Set(steps.slice(i + 1).map((x) => x.nodeId));
    for (const e of doc.edges) if (e.from === s.nodeId && e.port === s.port && later.has(e.to)) taken.add(edgeKey(e.from, e.port, e.to));
  });
  return { status, taken };
}

export const RUN_STATUS: Record<WorkflowRunStatus, { tone: BadgeTone; text: string }> = {
  running: { tone: 'running', text: 'Running' },
  waiting: { tone: 'waiting', text: 'Waiting' },
  succeeded: { tone: 'accent', text: 'Succeeded' },
  quiet: { tone: 'neutral', text: 'Nothing to do' },
  failed: { tone: 'failed', text: 'Failed' },
  cancelled: { tone: 'neutral', text: 'Cancelled' },
};

export function duration(ms: number): string {
  if (ms < 1000) return `${ms} ms`;
  const s = Math.round(ms / 1000);
  if (s < 60) return `${s} s`;
  const m = Math.round(s / 60);
  return m < 60 ? `${m} min` : `${Math.round(m / 60)} h`;
}
