import { useQuery, useQueryClient } from '@tanstack/react-query';
import { api } from '../api/client';

/** The Inbox, polled with the rest of the UI. The rail's count reads the same query. */
export function useInbox() {
  return useQuery({ queryKey: ['inbox'], queryFn: () => api.inbox(), refetchInterval: 2000 });
}

export function useRefreshInbox() {
  const qc = useQueryClient();
  return () => qc.invalidateQueries({ queryKey: ['inbox'] });
}
