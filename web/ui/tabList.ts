import type { KeyboardEvent } from 'react';

/**
 * Arrow-key navigation for a hand-built `role="tablist"` (put it on the list's `onKeyDown`): Left/Right move to the
 * previous/next tab (wrapping), Home/End to the first/last. The tab it lands on is focused and clicked, so it is
 * shown. Disabled tabs are skipped; keys typed anywhere other than on a tab (a rename field, a close button) pass
 * through. Give the tabs a roving `tabIndex` (`tabIndexOf`) so Tab enters the list once.
 */
export function tabListKeyDown(e: KeyboardEvent<HTMLElement>): void {
  if (e.altKey || e.ctrlKey || e.metaKey || e.shiftKey) return;
  const target = e.target as HTMLElement;
  if (target.getAttribute('role') !== 'tab') return;
  const tabs = [...e.currentTarget.querySelectorAll<HTMLElement>('[role="tab"]')].filter(
    (t) => t === target || !(t.hasAttribute('disabled') || t.getAttribute('aria-disabled') === 'true'),
  );
  const at = tabs.indexOf(target);
  if (at < 0) return;
  const n = tabs.length;
  const to = { ArrowLeft: (at - 1 + n) % n, ArrowRight: (at + 1) % n, Home: 0, End: n - 1 }[e.key];
  if (to === undefined) return;
  e.preventDefault();
  const next = tabs[to];
  if (!next || next === target) return;
  next.focus();
  next.click();
}

/** A roving `tabIndex`: the selected tab is the list's one Tab stop (the first tab when none is selected). */
export function tabIndexOf(selected: boolean, index: number, anySelected: boolean): 0 | -1 {
  return selected || (!anySelected && index === 0) ? 0 : -1;
}
