import { useCallback, useSyncExternalStore } from 'react';
import { useQueryClient, type QueryKey } from '@tanstack/react-query';

/**
 * When the cached query `key` last got an answer (0 before its first): changes on every poll, even an unchanged one.
 * Only the views that act on every poll (a terminal's reattach check) read it, deep in the tree, so the lists above
 * them re-render only when their data changes (PDT-136). It doesn't fetch: whoever polls the query does.
 */
export function usePolledAt(key: QueryKey): number {
  const qc = useQueryClient();
  const subscribe = useCallback((onChange: () => void) => qc.getQueryCache().subscribe(onChange), [qc]);
  return useSyncExternalStore(subscribe, () => qc.getQueryState(key)?.dataUpdatedAt ?? 0);
}
