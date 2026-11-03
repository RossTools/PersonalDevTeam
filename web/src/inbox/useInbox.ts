import { useQuery, useQueryClient } from '@tanstack/react-query';
import { api } from '../api/client';
import { POLL } from '../common/poll';

export const inboxKey = ['inbox'] as const;

/** The Inbox, polled with the rest of the UI. The rail's count reads the same query. */
export function useInbox() {
  return useQuery({ queryKey: inboxKey, queryFn: () => api.inbox(), refetchInterval: POLL.live });
}

export function useRefreshInbox() {
  const qc = useQueryClient();
  return () => qc.invalidateQueries({ queryKey: inboxKey });
}
