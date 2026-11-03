// Whether this page can reach pdtd (PDT-463): shown on every width, so a stale screen never looks healthy.
import { useSyncExternalStore } from 'react';
import { onlineManager } from '@tanstack/react-query';
import { Banner } from '@ui';

/** The browser's own online state, as React Query sees it (queries pause, without failing, while it is offline). */
function useOnline(): boolean {
  return useSyncExternalStore(onlineManager.subscribe.bind(onlineManager), () => onlineManager.isOnline());
}

/**
 * Whether the host answers: false while the browser is offline or the workspaces poll (every 2 s, on every screen)
 * has failed. A failed poll counts once React Query's retries are spent, a few seconds after pdtd stops.
 */
export function useConnected(pollError: unknown): boolean {
  return useOnline() && !pollError;
}

/** A query that has never loaded and has failed at least once: stays true across its refetches until data arrives. */
export function failedSoFar(q: { data: unknown; errorUpdateCount: number }): boolean {
  return q.data === undefined && q.errorUpdateCount > 0;
}

/** A failed strip above the main view while the host can't be reached; what is on screen may be out of date. */
export function ConnectionBanner({ connected }: { connected: boolean }) {
  if (connected) return null;
  // Short enough for a phone's width; the rest is in the tooltip.
  return <Banner tone="failed" title="What you see may be out of date. PDT keeps trying.">Can't reach the host; PDT keeps trying.</Banner>;
}
