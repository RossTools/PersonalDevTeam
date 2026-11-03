import { useState } from 'react';

/**
 * A list that loads `page` rows at a time: the `limit` to ask the server for, `more()` for the next page, and whether
 * `loaded` rows mean there may be more. It starts again at one page when `key` (the list's filters) changes.
 */
export function usePageLimit(page: number, key: string) {
  const [s, set] = useState({ key, limit: page });
  const limit = s.key === key ? s.limit : page;
  return {
    limit,
    more: () => set({ key, limit: limit + page }),
    hasMore: (loaded: number) => loaded >= limit,
  };
}
