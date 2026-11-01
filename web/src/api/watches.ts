import { enc, queryString, request } from './core';
import type { NewWatch } from './gen/NewWatch';
import type { WatchInfo } from './gen/WatchInfo';

/** Watches on tickets and workflow runs. */
export const watchesApi = {
  /** Watches (PDT-319): who hears about a ticket or a workflow run. */
  watches: {
    list: (q: { ticket?: string; run?: string; session?: string }) => request<WatchInfo[]>(`/api/watches${queryString(q)}`),
    /** The user's watch (one per target: watching again changes its events). */
    create: (w: NewWatch) => request<WatchInfo>('/api/watches', { method: 'POST', body: JSON.stringify(w) }),
    remove: (id: string) => request<void>(`/api/watches/${enc(id)}`, { method: 'DELETE' }),
  },
};
