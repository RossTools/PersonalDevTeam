// Workflows: their query keys, and what a write to one marks stale.
import type { QueryClient } from '@tanstack/react-query';
import type { WorkflowDetail } from '../api/gen/WorkflowDetail';

export const workflowKeys = {
  all: ['workflows'] as const,
  one: (id: string) => ['workflow', id] as const,
  versions: (id: string) => ['workflow-versions', id] as const,
  version: (id: string, version: number) => ['workflow-version', id, version] as const,
  diff: (id: string, from: number, to: number) => ['workflow-diff', id, from, to] as const,
  /** Every workflow's runs (`runs(id)` one workflow's). */
  allRuns: ['workflow-runs'] as const,
  runs: (id: string) => ['workflow-runs', id] as const,
  run: (runId: string) => ['workflow-run', runId] as const,
  templates: ['workflow-templates'] as const,
  template: (id: string) => ['workflow-template', id] as const,
  catalog: ['workflow-catalog'] as const,
  check: (text: string) => ['workflow-check', text] as const,
  export: (ids: string[]) => ['workflow-export', ids] as const,
};

/**
 * The Settings sections that show a built-in workflow: whether it is on and whether it is customized (Settings →
 * Agents, Notifications, Updates, Storage, PM).
 */
const BUILTIN_VIEWS = [['stuck-agents'], ['notifications', 'rules'], ['install-updates'], ['storage-cleanup'], ['pm-checkpoint']];

/** After a workflow is made, saved, copied or deleted: the list and the Settings sections that show a built-in. */
export function invalidateWorkflows(qc: QueryClient) {
  void qc.invalidateQueries({ queryKey: workflowKeys.all });
  for (const queryKey of BUILTIN_VIEWS) void qc.invalidateQueries({ queryKey });
}

/**
 * After a workflow is turned on or off: the list, its page, and the Settings sections that show a built-in (PDT-451),
 * whichever of them changed it.
 */
export function invalidateWorkflow(qc: QueryClient, id: string) {
  void qc.invalidateQueries({ queryKey: workflowKeys.one(id) });
  invalidateWorkflows(qc);
}

/** After a save that answers with the workflow as it is now: its page has it; the list and its versions refetch. */
export function savedWorkflow(qc: QueryClient, d: WorkflowDetail) {
  qc.setQueryData(workflowKeys.one(d.id), d);
  invalidateWorkflows(qc);
  void qc.invalidateQueries({ queryKey: workflowKeys.versions(d.id) });
}
