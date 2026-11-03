// A project's worktrees: one query per project, shared by the sidebar, the tabs, the place picker and Settings.
import type { QueryClient } from '@tanstack/react-query';
import { api } from '../api/client';
import { invalidateSessions } from '../sessions/queries';
import { invalidateWorkspaces } from '../workspaces/queries';
import { POLL } from './poll';

export const worktreeKeys = {
  all: ['worktrees'] as const,
  project: (projectId: string) => ['worktrees', projectId] as const,
};

/** The query for a project's worktrees (`useQuery({ ...worktreesQuery(id), enabled })`). */
export const worktreesQuery = (projectId: string) => ({
  queryKey: worktreeKeys.project(projectId),
  queryFn: () => api.listWorktrees(projectId),
  // Agents, the PM, auto-start and merge-then-finish make and remove worktrees without this UI asking (PDT-451).
  refetchInterval: POLL.slow,
});

/**
 * After a write that may make or remove a worktree: the project's list (every project's without one), and the sessions
 * and workspaces that live in worktrees.
 */
export function invalidateWorktrees(qc: QueryClient, projectId?: string) {
  void qc.invalidateQueries({ queryKey: projectId ? worktreeKeys.project(projectId) : worktreeKeys.all });
  void invalidateSessions(qc);
  void invalidateWorkspaces(qc);
}
