import type { QueryClient } from '@tanstack/react-query';

/**
 * The Settings sections that show a built-in workflow: whether it is on and whether it is customized (Settings →
 * Agents, Notifications, Updates, Storage, PM).
 */
const BUILTIN_VIEWS = [['stuck-agents'], ['notifications', 'rules'], ['install-updates'], ['storage-cleanup'], ['pm-checkpoint']];

/** After a workflow is saved (its page has the saved one): the list and the Settings sections that show a built-in. */
export function invalidateWorkflowViews(qc: QueryClient) {
  void qc.invalidateQueries({ queryKey: ['workflows'] });
  for (const queryKey of BUILTIN_VIEWS) void qc.invalidateQueries({ queryKey });
}

/**
 * After a workflow is turned on or off: the list, its page, and the Settings sections that show a built-in (PDT-451),
 * whichever of them changed it.
 */
export function invalidateWorkflow(qc: QueryClient, id: string) {
  void qc.invalidateQueries({ queryKey: ['workflow', id] });
  invalidateWorkflowViews(qc);
}
