import { useEffect } from 'react';
import { api } from '../api/client';

/** Tells the daemon the user is looking at this session (clears "unseen" on every device). It posts on mount, on
 * every poll (so something that happens while it is on screen is seen at once), and when the tab becomes visible,
 * but only while the tab is visible. */
export function useViewTracking(sessionId: string, polledAt?: number) {
  useEffect(() => {
    if (document.visibilityState === 'visible') void api.viewSession(sessionId).catch(() => {});
  }, [sessionId, polledAt]);
  useEffect(() => {
    const onVisible = () => {
      if (document.visibilityState === 'visible') void api.viewSession(sessionId).catch(() => {});
    };
    document.addEventListener('visibilitychange', onVisible);
    return () => document.removeEventListener('visibilitychange', onVisible);
  }, [sessionId]);
}
