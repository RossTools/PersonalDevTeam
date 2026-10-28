// A page loaded before PDT was updated still names the old build's chunks, which the server no longer has: opening a
// lazy view (a phone's Inbox item) then fails to import it, and without a reload the view can't show (PDT-209).

const KEY = 'pdt.reloadedForNewBuild';
/** Reloads at most this often, so a chunk that is really missing shows the error instead of reloading forever. */
const EVERY_MS = 30_000;

/** Whether `error` is a lazy chunk failing to load (Chrome and Firefox, and Safari's wording). */
export function isChunkLoadError(error: unknown): boolean {
  const message = error instanceof Error ? error.message : String(error);
  return /dynamically imported module|Importing a module script failed|error loading dynamically imported module/i.test(message);
}

/** Reloads the page to get the new build, unless it reloaded for that a moment ago. Says whether it reloads. */
export function reloadForNewBuild(storage: Storage = sessionStorage, reload: () => void = () => window.location.reload(), now = Date.now()): boolean {
  const last = Number(storage.getItem(KEY) ?? 0);
  if (now - last < EVERY_MS) return false;
  storage.setItem(KEY, String(now));
  reload();
  return true;
}

/** Vite reports a lazy chunk it couldn't load as `vite:preloadError`: reload into the new build instead of failing. */
export function reloadOnStaleChunks() {
  // The error still reaches the view's error boundary, which offers Reload if this one is skipped.
  window.addEventListener('vite:preloadError', () => void reloadForNewBuild());
}
