// Answers held for Undo (PDT-623): an Inbox answer goes out only once its few seconds pass, so Undo cancels it before
// anything happens, inside PDT or out (a review posted on a pull request, a ticket made). Until it has gone, the item
// is out of the Inbox's list; Undo puts it back. The page going away (closed, reloaded) ends the window: what is held
// goes out at once, with `keepalive` so it outlives the page (PDT-707), and a reload for a new build waits for it.
import { useSyncExternalStore } from 'react';
import { toast } from '@ui';
import { setLeaving } from '../api/core';

/** How long an answer waits for Undo (ms). Tests set it to 0 (test-setup.ts). */
export const holdTiming = { undoMs: 5000 };

/** An answer waiting to go out, or going. */
export interface HeldAnswer {
  itemId: string;
  /** The item's title. */
  title: string;
  /** What was answered ("Request changes", "Allow"). */
  label: string;
  /** When it goes out (ms). */
  sendsAt: number;
  /** It is going out: too late to undo. */
  sending: boolean;
}

interface Entry extends HeldAnswer {
  timer: ReturnType<typeof setTimeout>;
  send: () => Promise<unknown>;
}

const entries = new Map<string, Entry>();
const listeners = new Set<() => void>();
let snapshot: readonly HeldAnswer[] = [];
let ids: ReadonlySet<string> = new Set();

function changed() {
  snapshot = [...entries.values()].map(({ timer: _, send: __, ...h }) => h);
  ids = new Set(entries.keys());
  listeners.forEach((l) => l());
}

/**
 * Holds the answer to `itemId` for the Undo window, then runs `send` (which reports its own errors). The item stays out
 * of the list until `send` settles; a failed one comes back.
 */
export function holdAnswer(h: { itemId: string; title: string; label: string; send: () => Promise<unknown> }) {
  const old = entries.get(h.itemId);
  if (old) clearTimeout(old.timer);
  const timer = setTimeout(() => sendNow(h.itemId, timer), holdTiming.undoMs);
  entries.set(h.itemId, { itemId: h.itemId, title: h.title, label: h.label, sendsAt: Date.now() + holdTiming.undoMs, sending: false, timer, send: h.send });
  changed();
}

/** Sends the held answer `timer` was set for, unless it was undone or answered again since. */
function sendNow(itemId: string, timer: Entry['timer']) {
  const e = entries.get(itemId);
  if (!e || e.timer !== timer || e.sending) return;
  clearTimeout(timer);
  entries.set(itemId, { ...e, sending: true });
  changed();
  void e.send().catch(() => undefined).finally(() => {
    if (entries.get(itemId)?.timer === timer) entries.delete(itemId);
    changed();
  });
}

/** Sends every held answer now, without waiting out its Undo window (the page is going away). */
function sendHeldAnswers() {
  [...entries.values()].forEach((e) => sendNow(e.itemId, e.timer));
}

// Closing the tab or reloading ends the Undo window: the answers go out as the page goes, with `keepalive`. A page
// kept in the back/forward cache and shown again sends normally from then on.
if (typeof window !== 'undefined') {
  window.addEventListener('pagehide', () => {
    if (entries.size === 0) return;
    setLeaving(true);
    sendHeldAnswers();
  });
  window.addEventListener('pageshow', () => setLeaving(false));
}

/** Cancels a held answer that hasn't gone out: true when it did, and the item is back. */
export function undoAnswer(itemId: string): boolean {
  const e = entries.get(itemId);
  if (!e || e.sending) return false;
  clearTimeout(e.timer);
  entries.delete(itemId);
  changed();
  return true;
}

/** The toast with Undo for a held answer; `onUndo` once it is undone (show the item again). */
export function undoToast(itemId: string, title: string, label: string, onUndo?: () => void) {
  toast.info(label, {
    description: title,
    duration: holdTiming.undoMs || undefined,
    action: { label: 'Undo', onClick: () => { if (undoAnswer(itemId)) onUndo?.(); else toast.info('Too late to undo: it has gone out.'); } },
  });
}

const subscribe = (l: () => void) => {
  listeners.add(l);
  return () => listeners.delete(l);
};

/** The answers held or going out, oldest first. */
export function useHeldAnswers(): readonly HeldAnswer[] {
  return useSyncExternalStore(subscribe, () => snapshot);
}

/** The items whose answers are held or going out: the list leaves them out. */
export function useHeldIds(): ReadonlySet<string> {
  return useSyncExternalStore(subscribe, () => ids);
}

/** For tests: nothing held. */
export function resetHeldAnswers() {
  entries.forEach((e) => clearTimeout(e.timer));
  entries.clear();
  setLeaving(false);
  changed();
}
