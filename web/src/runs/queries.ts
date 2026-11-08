// Runs: a project's list (polled, shared by the tree, the tabs and the run's own view) and its run configuration.
import type { QueryClient } from '@tanstack/react-query';
import { api } from '../api/client';
import { POLL } from '../common/poll';
import { RECENT_ENDED } from '../sessions/queries';

export const runKeys = {
  all: ['runs'] as const,
  project: (projectId: string) => ['runs', projectId] as const,
  /** A project's run configuration, as a worktree (or, with null, the main checkout) reads it. */
  config: (projectId: string, worktreeId: string | null) => ['run-config', projectId, worktreeId] as const,
  /** Every checkout's run configuration of a project. */
  configs: (projectId: string) => ['run-config', projectId] as const,
  commands: (projectId: string) => ['project-commands', projectId] as const,
};

/** The query for a project's runs (`useQuery({ ...runsQuery(id), enabled })`). */
export const runsQuery = (projectId: string) => ({
  queryKey: runKeys.project(projectId),
  queryFn: () => api.listRuns(projectId, { ended: RECENT_ENDED }),
  refetchInterval: POLL.live,
});

/** After a write that starts, stops or removes a run: the project's list (every list without one). */
export function invalidateRuns(qc: QueryClient, projectId?: string) {
  return qc.invalidateQueries({ queryKey: projectId ? runKeys.project(projectId) : runKeys.all });
}

/** After the user approves commands: the project's list of them (Settings shows which wait on approval). */
export function invalidateCommands(qc: QueryClient, projectId: string) {
  return qc.invalidateQueries({ queryKey: runKeys.commands(projectId) });
}
