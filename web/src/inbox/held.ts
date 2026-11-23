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
  /** Its run of the window: a later answer to the same item, or Undo, makes a new one. */
  run: object;
  /** Null while the window is paused. */
  timer: ReturnType<typeof setTimeout> | null;
  /** What is left of the window while it is paused (ms). */
  left: number;
  send: () => Promise<unknown>;
  /** Its Undo toast, which lasts as long as Undo works. */
  toastId?: string | number;
}

const entries = new Map<string, Entry>();
const listeners = new Set<() => void>();
let snapshot: readonly HeldAnswer[] = [];
let ids: ReadonlySet<string> = new Set();
/** The Undo toasts are hovered or focused: the windows wait (PDT-667, WCAG 2.2.1). */
let paused = false;

function changed() {
  snapshot = [...entries.values()].map(({ itemId, title, label, sendsAt, sending }) => ({ itemId, title, label, sendsAt, sending }));
  ids = new Set(entries.keys());
  listeners.forEach((l) => l());
}

/** Takes `e`'s Undo toast away (with no id, Sonner would take every toast). */
function dismissToast(e: Entry) {
  if (e.toastId !== undefined) toast.dismiss(e.toastId);
}

/** Runs out what is left of `e`'s window, unless the windows are paused. */
function start(e: Entry) {
  if (e.timer) clearTimeout(e.timer);
  e.timer = paused ? null : setTimeout(() => sendNow(e.itemId, e.run), e.left);
  e.sendsAt = Date.now() + e.left;
}

/**
 * Holds the answer to `itemId` for the Undo window, then runs `send` (which reports its own errors). The item stays out
 * of the list until `send` settles; a failed one comes back.
 */
export function holdAnswer(h: { itemId: string; title: string; label: string; send: () => Promise<unknown> }) {
  const old = entries.get(h.itemId);
  if (old?.timer) clearTimeout(old.timer);
  if (old) dismissToast(old);
  const e: Entry = { itemId: h.itemId, title: h.title, label: h.label, sendsAt: 0, sending: false, run: {}, timer: null, left: holdTiming.undoMs, send: h.send };
  entries.set(h.itemId, e);
  start(e);
  changed();
}

/** Sends the held answer of `run`, unless it was undone or answered again since. */
function sendNow(itemId: string, run: object) {
  const e = entries.get(itemId);
  if (!e || e.run !== run || e.sending) return;
  if (e.timer) clearTimeout(e.timer);
  dismissToast(e);
  entries.set(itemId, { ...e, timer: null, sending: true });
  changed();
  void e.send().catch(() => undefined).finally(() => {
    if (entries.get(itemId)?.run === run) entries.delete(itemId);
    changed();
  });
}

/** Sends every held answer now, without waiting out its Undo window (the page is going away). */
function sendHeldAnswers() {
  [...entries.values()].forEach((e) => sendNow(e.itemId, e.run));
}

/** Pauses every held answer's window, or runs out what is left of each (at least a moment, to move off and back). */
function setPaused(p: boolean) {
  if (p === paused) return;
  paused = p;
  const now = Date.now();
  entries.forEach((e) => {
    if (e.sending) return;
    if (p) {
      if (e.timer) clearTimeout(e.timer);
      e.timer = null;
      e.left = Math.max(0, e.sendsAt - now);
    } else {
      e.left = Math.max(e.left, Math.min(holdTiming.undoMs, RESUME_MS));
      start(e);
    }
  });
  changed();
}

/** The least a window has left once the pointer or focus leaves its toast (ms). */
const RESUME_MS = 1500;

const inToasts = (t: EventTarget | null) => t instanceof Element && !!t.closest('[data-sonner-toaster]');

// Closing the tab or reloading ends the Undo window: the answers go out as the page goes, with `keepalive`. A page
// kept in the back/forward cache and shown again sends normally from then on.
if (typeof window !== 'undefined') {
  window.addEventListener('pagehide', () => {
    if (entries.size === 0) return;
    setLeaving(true);
    sendHeldAnswers();
  });
  window.addEventListener('pageshow', () => setLeaving(false));
  // While the pointer or focus is on a toast, Undo stays possible: the windows wait, as the toasts do.
  let hovered = false;
  let focused = false;
  const update = () => setPaused(hovered || focused);
  window.addEventListener('pointerover', (e) => { hovered = inToasts(e.target); update(); });
  window.addEventListener('pointerout', (e) => { if (!e.relatedTarget) { hovered = false; update(); } });
  window.addEventListener('focusin', (e) => { focused = inToasts(e.target); update(); });
  window.addEventListener('focusout', (e) => { if (!e.relatedTarget) { focused = false; update(); } });
}

/** Cancels a held answer that hasn't gone out: true when it did, and the item is back. */
export function undoAnswer(itemId: string): boolean {
  const e = entries.get(itemId);
  if (!e || e.sending) return false;
  if (e.timer) clearTimeout(e.timer);
  dismissToast(e);
  entries.delete(itemId);
  changed();
  return true;
}

/** Undoes the newest held answer that hasn't gone out (the Inbox's `u`): its item, or null when there is none. */
export function undoLast(): string | null {
  const last = [...entries.values()].filter((e) => !e.sending).at(-1);
  return last && undoAnswer(last.itemId) ? last.itemId : null;
}

/**
 * The toast with Undo for a held answer; `onUndo` once it is undone (show the item again). It lasts as long as Undo
 * works: it goes when the answer goes out.
 */
export function undoToast(itemId: string, title: string, label: string, onUndo?: () => void) {
  const e = entries.get(itemId);
  if (!e || e.sending) return;
  e.toastId = toast.info(label, {
    description: title,
    duration: Infinity,
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
  entries.forEach((e) => { if (e.timer) clearTimeout(e.timer); });
  entries.clear();
  paused = false;
  setLeaving(false);
  changed();
}
