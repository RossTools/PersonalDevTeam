import { useEffect, useRef } from 'react';
import { api } from '../api/client';

/**
 * Tells the daemon the user is looking at this session (clears "unseen" on every device). It posts on mount, when a
 * poll finds the session unseen (something happened while it is on screen, so it is seen at once), and when the tab
 * becomes visible, but only while the tab is visible. A poll that finds nothing new posts nothing (PDT-136: N open
 * panes were N writes every 2 s).
 */
export function useViewTracking(sessionId: string, unseen: boolean) {
  const posted = useRef<string | null>(null);
  useEffect(() => {
    if (posted.current === sessionId && !unseen) return;
    if (document.visibilityState !== 'visible') return;
    posted.current = sessionId;
    void api.viewSession(sessionId).catch(() => {});
  }, [sessionId, unseen]);
  useEffect(() => {
    const onVisible = () => {
      if (document.visibilityState === 'visible') void api.viewSession(sessionId).catch(() => {});
    };
    document.addEventListener('visibilitychange', onVisible);
    return () => document.removeEventListener('visibilitychange', onVisible);
  }, [sessionId]);
}
