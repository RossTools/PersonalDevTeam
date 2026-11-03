// Workspaces and their tabs: one query, shared by the shell, the tree and every tab action.
import type { QueryClient } from '@tanstack/react-query';
import { api } from '../api/client';
import { POLL } from '../common/poll';

export const workspaceKeys = {
  all: ['workspaces'] as const,
};

/** The query for every workspace (`useQuery(workspacesQuery)`). */
export const workspacesQuery = {
  queryKey: workspaceKeys.all,
  queryFn: api.listWorkspaces,
  refetchInterval: POLL.live,
};

/** After a write that opens, closes or renames a tab (a ticket tab's title follows its ticket). */
export function invalidateWorkspaces(qc: QueryClient) {
  return qc.invalidateQueries({ queryKey: workspaceKeys.all });
}
