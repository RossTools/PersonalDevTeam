import { useEffect, useRef } from 'react';
import { useQuery } from '@tanstack/react-query';
import { api } from '../api/client';
import type { Health } from '../api/gen/Health';
import { hostKeys } from '../host/queries';

/** This daemon's health: version, profile, port and whether it is a development instance. Fetched once here;
 * `useReloadOnNewBuild` keeps the same query polled for `webBuild`. */
export function useInstance() {
  return useQuery<Health>({ queryKey: hostKeys.health, queryFn: api.health, staleTime: Infinity });
}

/** The browser tab title: `(<n>) ` while the Inbox has n items (`useAttention`), `[dev] ` on a development instance. */
export function formatTitle(base: string, { attention = 0, dev = false }: { attention?: number; dev?: boolean }): string {
  return `${attention > 0 ? `(${attention}) ` : ''}${dev ? '[dev] ' : ''}${base}`;
}

/**
 * Owns `document.title`: the page title it found at mount, prefixed by [formatTitle] (`(2) [dev] PDT`). Restores
 * the original title on unmount.
 */
export function useDevTitle(attention = 0) {
  const dev = useInstance().data?.dev ?? false;
  const base = useRef(document.title);
  useEffect(() => {
    document.title = formatTitle(base.current, { attention, dev });
  }, [attention, dev]);
  useEffect(() => () => { document.title = base.current; }, []);
}
