import type { Notification } from '../api/gen/Notification';
import type { Show } from '../api/gen/Show';

/** Rows older than this on arrival (a laptop waking up) go to the Inbox but show no toast or desktop notification. */
export const STALE_MS = 2 * 60 * 1000;

const RANK: Record<Show, number> = { off: 0, dot: 1, inbox: 2, toast: 3, desktop: 4 };

/** Whether `show` reaches `at least`. */
export function reaches(show: Show, atLeast: Show): boolean {
  return RANK[show] >= RANK[atLeast];
}

/** What this page knows when a row arrives. */
export interface Present {
  /** The page is visible (not a background tab or minimised window). */
  visible: boolean;
  /** This device allowed desktop notifications (Settings → Notifications). */
  desktopAllowed: boolean;
  /** This tab shows desktop notifications for the browser (it holds the lock; other tabs stay quiet). */
  leader: boolean;
  /** The session in the focused tab, when the Sessions view shows one. */
  onScreenSession: string | null;
  /** The Inbox item open in the Inbox. */
  onScreenInbox: string | null;
  now: number;
}

/**
 * What a new or updated row does on this page (PDT-108): a toast while the page is visible (not for what is
 * already on screen), a desktop notification while it is hidden (only from the tab holding the lock, on a device that
 * allowed them), or nothing. Rows that arrive late show neither.
 */
export function presentation(n: Notification, p: Present): 'toast' | 'desktop' | null {
  if (n.clearedAt !== null || p.now - n.createdAt > STALE_MS) return null;
  if (p.visible) {
    if (!reaches(n.show, 'toast')) return null;
    if (n.sessionId && n.sessionId === p.onScreenSession) return null;
    if (n.inboxItemId && n.inboxItemId === p.onScreenInbox) return null;
    return 'toast';
  }
  return reaches(n.show, 'desktop') && p.desktopAllowed && p.leader ? 'desktop' : null;
}

/** Each row is shown once per time it was made or updated. */
export function shownKey(n: Notification): string {
  return `${n.id}:${n.createdAt}`;
}
