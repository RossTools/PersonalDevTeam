// What a view looked like when it was left (PDT-379): the file and scroll in Changes, a section, a toggle. Kept in
// memory and mirrored to sessionStorage, under a scope (the workspace tab the view is in, or the app view for screens
// that aren't tabs) and a name the view picks (with the ids of what it shows). Views read it when they mount and write
// it as the user interacts, so leaving and coming back shows the view as it was. A closed tab's entries are dropped.
import { createContext, useCallback, useContext, useLayoutEffect, useReducer, useRef, useState, type SetStateAction } from 'react';

const PREFIX = 'pdt.view:';
const SEP = '|';

const mem = new Map<string, unknown>();
/** Keys written since the last flush to sessionStorage. */
const dirty = new Set<string>();
let flushTimer: ReturnType<typeof setTimeout> | null = null;

/** The entry under `key`, or undefined. */
export function readViewState<T = unknown>(key: string): T | undefined {
  if (mem.has(key)) return mem.get(key) as T;
  try {
    const raw = sessionStorage.getItem(PREFIX + key);
    if (raw !== null) {
      const v = JSON.parse(raw) as T;
      mem.set(key, v);
      return v;
    }
  } catch {
    // Unreadable or blocked storage: nothing kept.
  }
  return undefined;
}

function flush() {
  flushTimer = null;
  for (const key of dirty) {
    try {
      if (mem.has(key)) sessionStorage.setItem(PREFIX + key, JSON.stringify(mem.get(key)));
      else sessionStorage.removeItem(PREFIX + key);
    } catch {
      // Storage blocked or full: kept in memory for this page only.
    }
  }
  dirty.clear();
}

/** Stores `value` under `key` (undefined removes it). Memory at once; sessionStorage shortly after (scrolls write often). */
export function writeViewState(key: string, value: unknown) {
  if (value === undefined) mem.delete(key);
  else mem.set(key, value);
  dirty.add(key);
  flushTimer ??= setTimeout(flush, 200);
}

const scopeOf = (key: string) => key.slice(0, key.indexOf(SEP));

function storedKeys(): string[] {
  const keys: string[] = [];
  try {
    for (let i = 0; i < sessionStorage.length; i++) {
      const k = sessionStorage.key(i);
      if (k?.startsWith(PREFIX)) keys.push(k.slice(PREFIX.length));
    }
  } catch {
    // Blocked storage: only memory to look at.
  }
  return keys;
}

/** Drops every entry whose scope `drop` picks (a closed tab's). */
export function forgetViewStates(drop: (scope: string) => boolean) {
  for (const key of new Set([...mem.keys(), ...storedKeys()])) {
    if (key.includes(SEP) && drop(scopeOf(key))) writeViewState(key, undefined);
  }
}

/** The scope of a workspace tab's views. */
export const tabScope = (tabId: string) => `tab:${tabId}`;

/** Drops the entries of tabs that are no longer open (`open`: every tab id in every workspace). */
export function pruneTabViewStates(open: Iterable<string>) {
  const keep = new Set([...open].map(tabScope));
  forgetViewStates((scope) => scope.startsWith('tab:') && !keep.has(scope));
}

/** Empties the store (tests). */
export function clearViewStates() {
  mem.clear();
  dirty.clear();
  if (flushTimer !== null) clearTimeout(flushTimer);
  flushTimer = null;
  try {
    for (const k of storedKeys()) sessionStorage.removeItem(PREFIX + k);
  } catch {
    // Blocked storage.
  }
}

/** The scope the views below keep their state in: the shell gives the tab's or the view's. */
export const ViewScopeContext = createContext('app');

/** The store's key for `name` in the current scope. */
export function useViewKey(name: string): string {
  return `${useContext(ViewScopeContext)}${SEP}${name}`;
}

/**
 * Like useState, kept in the view-state store under `name` (include the ids of what the view shows): a view that
 * unmounts and mounts again starts from what it had.
 */
export function useViewState<T>(name: string, initial: T | (() => T)): [T, (v: SetStateAction<T>) => void] {
  const key = useViewKey(name);
  const [fallback] = useState(initial);
  const [, rerender] = useReducer((n: number) => n + 1, 0);
  const stored = readViewState<T>(key);
  const value = stored === undefined ? fallback : stored;
  const set = useCallback((v: SetStateAction<T>) => {
    const cur = readViewState<T>(key);
    const prev = cur === undefined ? fallback : cur;
    writeViewState(key, typeof v === 'function' ? (v as (p: T) => T)(prev) : v);
    rerender();
  }, [key, fallback]);
  return [value, set];
}

/** How long a restored scroll waits for content still loading to make room for it. */
const RESTORE_MS = 2000;

/**
 * Remembers a scrolling element's position under `name`: give `ref` to the element and `onScroll` to its scroll
 * event. The position comes back each time the element appears, once `ready`; content that loads after that (a
 * timeline, images) is waited for a moment, unless the user scrolls first.
 */
export function useRememberedScroll<E extends HTMLElement = HTMLDivElement>(name: string, ready = true) {
  const key = useViewKey(name);
  const ref = useRef<E>(null);
  // The element (and key) last restored, and a restore still waiting for room.
  const restored = useRef<{ el: E; key: string } | null>(null);
  const waiting = useRef<(() => void) | null>(null);
  useLayoutEffect(() => {
    const el = ref.current;
    if (!ready || !el || (restored.current?.el === el && restored.current.key === key)) return;
    waiting.current?.();
    restored.current = { el, key };
    const top = readViewState<number>(key);
    if (typeof top !== 'number') return;
    el.scrollTop = top;
    if (el.scrollTop >= top - 1 || typeof ResizeObserver === 'undefined') return;
    const until = Date.now() + RESTORE_MS;
    const ro = new ResizeObserver(() => {
      if (Date.now() > until) stop();
      else {
        el.scrollTop = top;
        if (el.scrollTop >= top - 1) stop();
      }
    });
    const stop = () => {
      ro.disconnect();
      for (const e of USER_SCROLLS) el.removeEventListener(e, stop);
      if (waiting.current === stop) waiting.current = null;
    };
    for (const c of el.children) ro.observe(c);
    for (const e of USER_SCROLLS) el.addEventListener(e, stop, { passive: true });
    waiting.current = stop;
  });
  useLayoutEffect(() => () => waiting.current?.(), []);
  const onScroll = useCallback(() => {
    const el = ref.current;
    if (!el || restored.current?.el !== el || waiting.current) return;
    writeViewState(key, el.scrollTop);
  }, [key]);
  return { ref, onScroll };
}

/** What the user does to scroll (a restore waiting for room gives way to them). */
const USER_SCROLLS = ['wheel', 'touchstart', 'mousedown', 'keydown'] as const;
