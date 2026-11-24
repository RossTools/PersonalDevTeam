import { useMemo } from 'react';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { api } from '../api/client';
import { POLL } from '../common/poll';
import { useHeldIds } from './held';
import { inboxKeys } from './queries';


/**
 * The Inbox, polled with the rest of the UI. The rail's count reads the same query. Items whose answers wait out
 * their Undo window (PDT-623) are left out, as if answered.
 */
export function useInbox() {
  const q = useQuery({ queryKey: inboxKeys.list, queryFn: () => api.inbox(), refetchInterval: POLL.live });
  const held = useHeldIds();
  const { data } = q;
  const shown = useMemo(() => {
    if (!data || held.size === 0) return data;
    const items = data.items.filter((i) => !held.has(i.id));
    return { ...data, items, total: Math.max(0, data.total - (data.items.length - items.length)) };
  }, [data, held]);
  return shown === data ? q : { ...q, data: shown };
}

export function useRefreshInbox() {
  const qc = useQueryClient();
  return () => qc.invalidateQueries({ queryKey: inboxKeys.all });
}
