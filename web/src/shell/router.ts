// The browser history behind the app's routes (PDT-111): the current location as React state, and navigation.
import { useEffect, useSyncExternalStore } from 'react';

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

/** Shows a path: a new history entry (Back returns), or with `replace` in place of the current one. */
export function navigate(to: string, opts?: { replace?: boolean }) {
  if (to === currentLocation()) return;
  if (opts?.replace || pushedThisTask) {
    window.history.replaceState(null, '', to);
  } else {
    window.history.pushState(null, '', to);
    pushedThisTask = true;
    setTimeout(() => { pushedThisTask = false; }, 0);
  }
  notify();
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
