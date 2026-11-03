// Sessions: a project's list (polled, shared by the tree, the tabs and the session's own view) and one session.
import type { QueryClient } from '@tanstack/react-query';
import { api } from '../api/client';
import { POLL } from '../common/poll';

export const sessionKeys = {
  all: ['sessions'] as const,
  project: (projectId: string) => ['sessions', projectId] as const,
  one: (id: string) => ['session', id] as const,
  lastEvents: (id: string) => ['session-last-events', id] as const,
  file: (id: string, path: string) => ['session-file', id, path] as const,
};

/** The query for a project's sessions (`useQuery({ ...sessionsQuery(id), enabled })`). */
export const sessionsQuery = (projectId: string) => ({
  queryKey: sessionKeys.project(projectId),
  queryFn: () => api.listSessions(projectId),
  refetchInterval: POLL.live,
});

/** After a write that starts, changes or ends a session: the project's list (every list without one). */
export function invalidateSessions(qc: QueryClient, projectId?: string) {
  return qc.invalidateQueries({ queryKey: projectId ? sessionKeys.project(projectId) : sessionKeys.all });
}
