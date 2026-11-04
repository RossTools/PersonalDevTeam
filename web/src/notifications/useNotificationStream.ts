import { createElement, useEffect, useRef } from 'react';
import { CircleHelp } from 'lucide-react';
import { useQueryClient } from '@tanstack/react-query';
import { toast } from '@ui/Toaster';
import type { Notification } from '../api/gen/Notification';
import type { NotificationTarget } from '../api/gen/NotificationTarget';
import { desktopNotificationsOn, holdLeadership, showDesktop } from './desktop';
import { presentation, shownKey } from './model';
import { inboxKey } from '../inbox/useInbox';
import { setStreamOpen } from '../common/poll';

const STREAM_PATH = '/api/notifications/stream';

/** What's on screen now (read when a row arrives). */
export interface OnScreen {
  session: string | null;
  inbox: string | null;
}

/**
 * Follows `GET /api/notifications/stream` (Server-Sent Events, PDT-108): every row refreshes the Inbox, and a new
 * one shows as a toast or, while the page is hidden, a desktop notification, as its Show and this device allow
 * (`presentation`). A cleared row closes its desktop notification. EventSource reconnects by itself and sends
 * `Last-Event-ID`, so a dropped connection gets what it missed. While it is connected the Updates poll slowly. `open` opens a row's target.
 */
export function useNotificationStream(open: (t: NotificationTarget) => void, onScreen: OnScreen) {
  const qc = useQueryClient();
  const latest = useRef({ open, onScreen });
  useEffect(() => {
    latest.current = { open, onScreen };
  }, [open, onScreen]);
  useEffect(() => {
    if (typeof EventSource === 'undefined') return;
    let leader = false;
    const letGo = holdLeadership(() => { leader = true; });
    const shown = new Set<string>();
    const desktop = new Map<string, globalThis.Notification>();
    const source = new EventSource(STREAM_PATH);
    const refresh = () => {
      void qc.invalidateQueries({ queryKey: inboxKey });
      void qc.invalidateQueries({ queryKey: ['notifications'] });
    };
    // While it is connected the stream brings every change, so the Updates poll slows down (`streamPoll`); when it
    // connects or drops, a refetch catches up and picks the new interval.
    const connected = (open: boolean) => {
      setStreamOpen(open);
      refresh();
    };
    source.onopen = () => connected(true);
    source.onerror = () => {
      if (source.readyState !== EventSource.OPEN) connected(false);
    };
    const arrived = (e: MessageEvent<string>) => {
      let n: Notification;
      try {
        n = JSON.parse(e.data) as Notification;
      } catch {
        return;
      }
      refresh();
      if (n.clearedAt !== null) {
        desktop.get(n.dedupeKey)?.close();
        desktop.delete(n.dedupeKey);
        return;
      }
      const key = shownKey(n);
      if (shown.has(key)) return;
      const now = latest.current.onScreen;
      const how = presentation(n, {
        visible: document.visibilityState === 'visible',
        desktopAllowed: desktopNotificationsOn(),
        leader,
        onScreenSession: now.session,
        onScreenInbox: now.inbox,
        now: Date.now(),
      });
      if (!how) return;
      shown.add(key);
      const click = () => latest.current.open(n.target);
      if (how === 'toast') {
        const action = n.target.kind === 'none' ? undefined : { label: 'Open', onClick: click };
        const shownAs = n.severity === 'error' ? toast.error : n.severity === 'warning' ? toast.info : toast.success;
        // A question asks: a "?", not a success tick (PDT-576).
        const icon = n.kind === 'inbox.question' ? createElement(CircleHelp, { className: 'size-4 text-link' }) : undefined;
        shownAs(n.title, { description: n.body || undefined, action, icon });
      } else {
        const d = showDesktop(n.title, n.body, n.dedupeKey, click);
        if (d) desktop.set(n.dedupeKey, d);
      }
    };
    for (const type of ['created', 'updated', 'cleared']) source.addEventListener(type, arrived as EventListener);
    return () => {
      source.close();
      setStreamOpen(false);
      letGo();
      for (const d of desktop.values()) d.close();
    };
  }, [qc]);
}
