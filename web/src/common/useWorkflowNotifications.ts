import { useEffect, useRef } from 'react';
import { toast } from '@ui/Toaster';
import { api } from '../api/client';
import type { Notified } from '../api/gen/Notified';
import { looksLikeKey } from '../tickets/model';

const PREF = 'pdt.desktopNotifications';
const POLL_MS = 2000;

/** Whether this browser shows workflow notifications on the desktop while PDT's page is hidden. */
export function desktopNotificationsOn(): boolean {
  return typeof localStorage !== 'undefined' && localStorage.getItem(PREF) === '1'
    && typeof Notification !== 'undefined' && Notification.permission === 'granted';
}

/** Turns desktop notifications on (asking the browser's permission) or off. Resolves to whether they are on. */
export async function setDesktopNotifications(on: boolean): Promise<boolean> {
  if (!on || typeof Notification === 'undefined') {
    localStorage.removeItem(PREF);
    return false;
  }
  const permission = Notification.permission === 'default' ? await Notification.requestPermission() : Notification.permission;
  if (permission !== 'granted') {
    localStorage.removeItem(PREF);
    return false;
  }
  localStorage.setItem(PREF, '1');
  return true;
}

/**
 * Shows each workflow `notify` step (a `pdt.notified` event, crates/pdt-api/src/workflow/nodes/notify.rs) as a toast,
 * and on the desktop while the page is hidden if the user turned that on. Only notifications from after the page
 * loaded. `openTicket` opens a ticket by its key.
 */
export function useWorkflowNotifications(openTicket: (key: string) => void) {
  const open = useRef(openTicket);
  useEffect(() => {
    open.current = openTicket;
  }, [openTicket]);
  useEffect(() => {
    let cursor: number | null = null;
    let stopped = false;
    const show = (n: Notified) => {
      const link = n.link ?? null;
      const action = link
        ? {
            label: 'Open',
            onClick: () => {
              if (looksLikeKey(link)) open.current(link);
              else window.open(link, '_blank', 'noopener');
            },
          }
        : undefined;
      const description = [n.body, `From ${n.workflowName}`].filter(Boolean).join(' · ');
      const shown = n.level === 'error' ? toast.error : n.level === 'warning' ? toast.info : toast.success;
      shown(n.title, { description, action });
      if (document.hidden && desktopNotificationsOn()) {
        const d = new Notification(n.title, { body: n.body || n.workflowName });
        d.onclick = () => {
          window.focus();
          action?.onClick();
        };
      }
    };
    const poll = async () => {
      try {
        if (cursor === null) {
          const newest = await api.listLogEvents({ type: 'pdt.notified', limit: 1 });
          cursor = newest[0]?.seq ?? 0;
          return;
        }
        const events = await api.listLogEvents({ type: 'pdt.notified', after: cursor, limit: 50 });
        for (const e of events) {
          cursor = Math.max(cursor, e.seq);
          if (!stopped) show(e.payload as Notified);
        }
      } catch {
        // The next poll tries again.
      }
    };
    void poll();
    const t = window.setInterval(() => void poll(), POLL_MS);
    return () => {
      stopped = true;
      window.clearInterval(t);
    };
  }, []);
}
