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

/**
 * How many ended sessions (and finished runs) a project's list brings beyond the running ones and those a tab holds:
 * workflow steps leave an ended session each, and the list is polled every 2 s.
 */
export const RECENT_ENDED = 50;

/** The query for a project's sessions (`useQuery({ ...sessionsQuery(id), enabled })`). */
export const sessionsQuery = (projectId: string) => ({
  queryKey: sessionKeys.project(projectId),
  queryFn: () => api.listSessions(projectId, { ended: RECENT_ENDED }),
  refetchInterval: POLL.live,
});

/** After a write that starts, changes or ends a session: the project's list (every list without one). */
export function invalidateSessions(qc: QueryClient, projectId?: string) {
  return qc.invalidateQueries({ queryKey: projectId ? sessionKeys.project(projectId) : sessionKeys.all });
}
