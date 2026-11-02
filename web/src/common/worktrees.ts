// A project's worktrees: one query per project, shared by the sidebar, the tabs, the place picker and Settings.
import type { QueryClient } from '@tanstack/react-query';
import { api } from '../api/client';

/**
 * How often the lists refetch: agents, the PM, auto-start and merge-then-finish make and remove worktrees without this
 * UI asking, so a slow poll shows them (PDT-451). The UI's own writes invalidate them at once.
 */
export const WORKTREE_POLL = 10_000;

export const worktreeKeys = {
  all: ['worktrees'] as const,
  project: (projectId: string) => ['worktrees', projectId] as const,
};

/** The query for a project's worktrees (`useQuery({ ...worktreesQuery(id), enabled })`). */
export const worktreesQuery = (projectId: string) => ({
  queryKey: worktreeKeys.project(projectId),
  queryFn: () => api.listWorktrees(projectId),
  refetchInterval: WORKTREE_POLL,
});

/** After a write that may make or remove a worktree: the project's list (every project's without one). */
export function invalidateWorktrees(qc: QueryClient, projectId?: string) {
  void qc.invalidateQueries({ queryKey: projectId ? worktreeKeys.project(projectId) : worktreeKeys.all });
}
