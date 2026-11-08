import { keepPreviousData, useQueries } from '@tanstack/react-query';
import { api } from '../api/client';
import { useDebounced } from '../common/useDebounced';
import { MIN_FILE_QUERY, type FileCheckout, type FoundFile } from './paletteItems';
import { fileKeys } from '../files/queries';

/** Files listed at most, across checkouts. */
const MAX_FOUND = 20;

/** Takes from each list in turn, so every checkout's best matches make the cut. */
export function interleave<T>(lists: T[][], max: number): T[] {
  const out: T[] = [];
  for (let i = 0; out.length < max && lists.some((l) => i < l.length); i++) {
    for (const l of lists) if (i < l.length && out.length < max) out.push(l[i]);
  }
  return out;
}

/**
 * The palette's files matching `query` by name in `checkouts`, once typing pauses and the query has 2 characters.
 * Each checkout is asked on its own, so a slow one holds up neither the others nor the rest of the palette.
 */
export function useFileFind(checkouts: FileCheckout[], query: string, enabled: boolean): FoundFile[] {
  const typed = query.trim();
  const q = useDebounced(typed);
  const asking = enabled && typed.length >= MIN_FILE_QUERY && q.length >= MIN_FILE_QUERY;
  const results = useQueries({
    queries: checkouts.map((c) => ({
      queryKey: fileKeys.find(c.projectId, c.worktreeId, q),
      queryFn: ({ signal }: { signal: AbortSignal }) => api.findFiles(c.projectId, c.worktreeId, q, signal),
      enabled: asking,
      placeholderData: keepPreviousData,
      staleTime: 5000,
    })),
  });
  if (!asking) return [];
  return interleave(results.map((r, i) => (r.data ?? []).map((path) => ({ checkout: checkouts[i], path }))), MAX_FOUND);
}
