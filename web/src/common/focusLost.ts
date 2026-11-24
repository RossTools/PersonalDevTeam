import { useCallback, useEffect, useRef } from 'react';

/**
 * Whether keyboard focus has nowhere to be: the focused control went away (a card answered, a button that hid itself)
 * and the browser dropped focus on the body. Callers then move it somewhere that still means something, but never take
 * it from where the user put it.
 */
export function focusLost(): boolean {
  const el = document.activeElement;
  return el === null || el === document.body;
}

type Refocus = { from: Element; to: () => HTMLElement | null | undefined };

/**
 * For a control that removes itself (Unsnooze, Remove, the last watcher's X): call `keep(from, to)` as it acts. Once
 * `from` (the control, or its row) has left the page and focus fell to the body, focus goes to what `to` finds then:
 * the next row, a heading, the control that stays (looking for a few frames, while it renders). Focus the user put elsewhere stays there. It also settles when the
 * component itself goes (its whole section went with the row), so `to` looks things up in the page, not in refs.
 */
export function useKeepFocus(): (from: Element | null | undefined, to: Refocus['to']) => void {
  const pending = useRef<Refocus | null>(null);
  const settle = useCallback(() => {
    const p = pending.current;
    if (!p || p.from.isConnected) return;
    pending.current = null;
    // What takes focus may render a pass later (a List's new rows do): look for a few frames.
    let tries = 10;
    const look = () => {
      if (!focusLost()) return;
      const to = p.to();
      if (to) to.focus({ preventScroll: true });
      else if (--tries > 0) requestAnimationFrame(look);
    };
    look();
  }, []);
  // Every render: the row goes in whichever refresh lands, and settling is idempotent.
  useEffect(settle);
  useEffect(() => settle, [settle]);
  return useCallback((from, to) => {
    pending.current = from ? { from, to } : null;
  }, []);
}

/** The rows of the List (React Aria's grid) under `root`, in order. */
export function listRows(root: ParentNode | null | undefined): HTMLElement[] {
  return root ? [...root.querySelectorAll<HTMLElement>('[role="row"][data-key]')] : [];
}

/**
 * For `useKeepFocus`, in a plain list: the first button of the item that takes the removed item's place (the one
 * before, when it was the last), else what `fallback` finds (the list went with it).
 */
export function itemInPlaceOf(item: Element | null, fallback?: () => HTMLElement | null | undefined): () => HTMLElement | null | undefined {
  const list = item?.parentElement;
  const at = list ? [...list.children].indexOf(item!) : 0;
  return () => {
    const items = list?.isConnected ? [...list.children] : [];
    return items[Math.min(at, items.length - 1)]?.querySelector<HTMLElement>('button') ?? fallback?.();
  };
}
