import { useEffect, useRef } from 'react';

/** True on macOS (and iPadOS with a keyboard, which reports itself as a Mac). */
export function isMac(): boolean {
  const nav = navigator as Navigator & { userAgentData?: { platform?: string } };
  const platform = nav.userAgentData?.platform || nav.platform || '';
  return /mac/i.test(platform);
}

/**
 * Calls `onOpen` for ⌘K on macOS, caught on the window in the capture phase so a focused terminal never sees it. Does
 * nothing elsewhere: Ctrl+K is readline's kill-line in the terminal, so other platforms use Ctrl+] k only.
 */
export function useMetaK(onOpen: () => void) {
  const handler = useRef(onOpen);
  handler.current = onOpen;
  useEffect(() => {
    if (!isMac()) return;
    const onKey = (e: KeyboardEvent) => {
      if (!e.metaKey || e.ctrlKey || e.altKey || e.shiftKey) return;
      if (e.key.toLowerCase() !== 'k' && e.code !== 'KeyK') return;
      e.preventDefault();
      e.stopPropagation();
      handler.current();
    };
    window.addEventListener('keydown', onKey, true);
    return () => window.removeEventListener('keydown', onKey, true);
  }, []);
}
