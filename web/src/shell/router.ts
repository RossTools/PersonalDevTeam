// The browser history behind the app's routes (PDT-111): the current location as React state, and navigation.
import { useEffect, useRef, useSyncExternalStore } from 'react';

const listeners = new Set<() => void>();
const notify = () => listeners.forEach((l) => l());

function subscribe(l: () => void) {
  listeners.add(l);
  if (listeners.size === 1) window.addEventListener('popstate', notify);
  return () => {
    listeners.delete(l);
    if (listeners.size === 0) window.removeEventListener('popstate', notify);
  };
}

/** The current path and query string. */
export const currentLocation = () => window.location.pathname + window.location.search;

// A click that switches the view and selects something in it navigates twice in one go: the second replaces the
// first, so Back walks through what was opened rather than each step of opening it.
let pushedThisTask = false;

/**
 * What the app keeps in each history entry: `n`, how many app entries lie behind it (0 on the page it was loaded
 * on), so ← knows whether Back stays in the app; `overlay`, an entry an open drawer added so Back closes it.
 */
interface EntryState {
  n?: number;
  overlay?: boolean;
}

const entry = (): EntryState => (window.history.state as EntryState | null) ?? {};

/**
 * Shows a path: a new history entry (Back returns), or with `replace` in place of the current one. An open drawer's
 * entry is replaced, so choosing in a drawer leaves one entry.
 */
export function navigate(to: string, opts?: { replace?: boolean }) {
  if (to === currentLocation() && !entry().overlay) return;
  whenAllowed(() => navigateNow(to, opts));
}

function navigateNow(to: string, opts?: { replace?: boolean }) {
  const cur = entry();
  if (opts?.replace || pushedThisTask || cur.overlay) {
    window.history.replaceState({ n: cur.n ?? 0 }, '', to);
  } else {
    window.history.pushState({ n: (cur.n ?? 0) + 1 }, '', to);
    pushedThisTask = true;
    setTimeout(() => { pushedThisTask = false; }, 0);
  }
  notify();
}

/** What may hold up leaving the screen (unsaved edits): asks, and resolves true to leave. */
let guard: (() => Promise<boolean>) | null = null;

/** Runs `go` (something that leaves the screen) now, or once the guard, when there is one, says so. */
export function whenAllowed(go: () => void) {
  if (!guard) return go();
  void guard().then((ok) => { if (ok) go(); });
}

/**
 * While `active` (unsaved changes), navigating in the app first asks `ask` (a confirm), and closing or reloading the
 * page asks the browser's own question. Browser Back is not held.
 */
export function useNavigationGuard(active: boolean, ask: () => Promise<boolean>) {
  const askRef = useRef(ask);
  askRef.current = ask;
  useEffect(() => {
    if (!active) return;
    const mine = () => askRef.current();
    guard = mine;
    const onUnload = (e: BeforeUnloadEvent) => e.preventDefault();
    window.addEventListener('beforeunload', onUnload);
    return () => {
      if (guard === mine) guard = null;
      window.removeEventListener('beforeunload', onUnload);
    };
  }, [active]);
}

/** ←: Back, when it stays in the app; otherwise (a detail screen opened from a link) `up` in place of this one. */
export function goBack(up: string) {
  if ((entry().n ?? 0) > 0) whenAllowed(() => window.history.back());
  else navigate(up, { replace: true });
}

let overlays = 0;

/**
 * While `open`, the history has an entry for it, so Back (the browser's, a phone's gesture) calls `close` rather than
 * leaving the screen. Closed another way, the entry is taken back off.
 */
export function useBackCloses(open: boolean, close: () => void) {
  const closeRef = useRef(close);
  closeRef.current = close;
  useEffect(() => {
    if (!open) return;
    overlays++;
    if (!entry().overlay) window.history.pushState({ n: (entry().n ?? 0) + 1, overlay: true }, '', currentLocation());
    const onPop = () => {
      if (!entry().overlay) closeRef.current();
    };
    window.addEventListener('popstate', onPop);
    return () => {
      overlays--;
      window.removeEventListener('popstate', onPop);
      // After the effect re-runs (React's dev double run) nothing is undone; a navigation already replaced the entry.
      setTimeout(() => {
        if (overlays === 0 && entry().overlay) window.history.back();
      }, 0);
    };
  }, [open]);
}

/** The current path and query string, re-rendering on navigation and on Back/Forward. */
export function useLocation(): string {
  return useSyncExternalStore(subscribe, currentLocation);
}

/** A path inside the app (not the API, not another site). */
export const isAppPath = (href: string) => href.startsWith('/') && !href.startsWith('//') && !/^\/api(\/|$)/.test(href);

/**
 * Plain clicks on links to the app's own paths (ticket keys in a conversation, Markdown links) navigate here instead
 * of reloading the page. Modified clicks and links with a target still open a new tab.
 */
export function useAppLinks() {
  useEffect(() => {
    const onClick = (e: MouseEvent) => {
      if (e.defaultPrevented || e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) return;
      const a = (e.target as Element | null)?.closest?.('a');
      if (!a || (a.target && a.target !== '_self') || a.hasAttribute('download')) return;
      const href = a.getAttribute('href');
      if (!href || !isAppPath(href)) return;
      e.preventDefault();
      navigate(href);
    };
    document.addEventListener('click', onClick);
    return () => document.removeEventListener('click', onClick);
  }, []);
}
