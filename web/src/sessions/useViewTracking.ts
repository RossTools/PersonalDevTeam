import { useEffect, useRef } from 'react';
import { api } from '../api/client';

/**
 * Tells the daemon the user is looking at this session (clears "unseen" on every device). It posts on mount, when a
 * poll finds unseen attention it hasn't cleared yet (something happened while it is on screen, so it is seen at once;
 * `unseenAt` is the session's `attentionAt` while it is unseen, else null), and when the tab becomes visible, but only
 * while the tab is visible. A poll that finds nothing new posts nothing (PDT-136: N open panes were N writes every 2 s).
 */
export function useViewTracking(sessionId: string, unseenAt: number | null) {
  const posted = useRef<string | null>(null);
  useEffect(() => {
    if (posted.current === sessionId && unseenAt === null) return;
    if (document.visibilityState !== 'visible') return;
    posted.current = sessionId;
    void api.viewSession(sessionId).catch(() => {});
  }, [sessionId, unseenAt]);
  useEffect(() => {
    const onVisible = () => {
      if (document.visibilityState === 'visible') void api.viewSession(sessionId).catch(() => {});
    };
    document.addEventListener('visibilitychange', onVisible);
    return () => document.removeEventListener('visibilitychange', onVisible);
  }, [sessionId]);
}
