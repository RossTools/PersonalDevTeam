import { enc, queryString, request } from './core';
import type { NewWatch } from './gen/NewWatch';
import type { WatchInfo } from './gen/WatchInfo';

/** Watches on tickets, workflow runs and documents. */
export const watchesApi = {
  /** Watches (PDT-319): who hears about a ticket, a workflow run or a document. */
  watches: {
    list: (q: { ticket?: string; run?: string; document?: string; session?: string }) => request<WatchInfo[]>(`/api/watches${queryString(q)}`),
    /** The user's watch (one per target: watching again changes its events). */
    create: (w: NewWatch) => request<WatchInfo>('/api/watches', { method: 'POST', body: JSON.stringify(w) }),
    remove: (id: string) => request<void>(`/api/watches/${enc(id)}`, { method: 'DELETE' }),
  },
};
