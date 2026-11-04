import { useEffect, useRef, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { api } from '../api/client';
import { hostKeys } from './queries';
import { POLL } from '../common/poll';

/** How often /api/health is asked for the web build; a restarted dev instance is picked up within this. */
const WEB_BUILD_POLL_MS = POLL.steady;

const reloadPage = () => location.reload();

/**
 * The build this page's code came from: the `web-build` meta a production build writes into index.html
 * (web/vite.config.ts), which pdtd's health reports as `webBuild` too. Null under the Vite dev server.
 */
export function servedBuild(doc: Document = document): string | null {
  return doc.querySelector<HTMLMetaElement>('meta[name="web-build"]')?.content || null;
}

/**
 * Reloads the page once pdtd serves a different web build than the one this page's code came from. Every dev build
 * reports the same version, so a dev instance restarted with a rebuilt web/dist is only seen here (version changes
 * are `useReloadOnNewVersion`'s, and a release changes the build too). The page compares against [servedBuild]; a page
 * without one (the Vite dev server) takes the first answer it sees. Health is asked again as soon as the page comes
 * back (a phone restoring a frozen or cached tab), not on the next poll. Waits while [hasUnsavedInput] says reloading
 * would lose something and tries again on the next poll; `held` is true meanwhile, for a banner offering the reload.
 */
export function useReloadOnNewBuild(reload: () => void = reloadPage, own: string | null = servedBuild()) {
  // Shares the ['health'] query with useInstance, so the page asks once at load and this observer keeps it fresh.
  const health = useQuery({ queryKey: hostKeys.health, queryFn: api.health, refetchInterval: WEB_BUILD_POLL_MS });
  const build = health.data?.webBuild;
  const loaded = useRef<string | null>(own);
  const [held, setHeld] = useState(false);
  const { refetch } = health;
  useEffect(() => {
    const check = () => void refetch();
    const onVisible = () => { if (document.visibilityState === 'visible') check(); };
    const onPageShow = (e: PageTransitionEvent) => { if (e.persisted) check(); };
    document.addEventListener('visibilitychange', onVisible);
    window.addEventListener('focus', check);
    window.addEventListener('online', check);
    window.addEventListener('pageshow', onPageShow);
    return () => {
      document.removeEventListener('visibilitychange', onVisible);
      window.removeEventListener('focus', check);
      window.removeEventListener('online', check);
      window.removeEventListener('pageshow', onPageShow);
    };
  }, [refetch]);
  // Every poll checks again (dataUpdatedAt changes even when the answer does not), so a held-back reload happens soon
  // after the field is cleared or the dialog closes.
  useEffect(() => {
    if (!build) return;
    if (!loaded.current) loaded.current = build;
    if (build === loaded.current) setHeld(false);
    else if (hasUnsavedInput()) setHeld(true);
    else reload();
  }, [build, health.dataUpdatedAt, reload]);
  return { held, reload };
}

/**
 * True while reloading would throw away something on screen: a modal dialog (a form) is open, or the focused field
 * holds text. The terminal's hidden input never counts; composer drafts are kept per browser, but reloading under the
 * user's fingers would still drop the caret mid-sentence.
 */
export function hasUnsavedInput(doc: Document = document): boolean {
  if (doc.querySelector('[aria-modal="true"], dialog[open]')) return true;
  const el = doc.activeElement;
  if (!(el instanceof HTMLElement) || el.classList.contains('xterm-helper-textarea')) return false;
  if (el instanceof HTMLTextAreaElement) return el.value !== '';
  if (el instanceof HTMLInputElement) return TEXT_INPUTS.has(el.type) && el.value !== '';
  return el.matches('[contenteditable]:not([contenteditable="false"])') && (el.textContent ?? '') !== '';
}

const TEXT_INPUTS = new Set(['text', 'search', 'url', 'email', 'tel', 'password', 'number']);
