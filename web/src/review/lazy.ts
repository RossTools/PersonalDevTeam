import { useEffect, useState, type RefObject } from 'react';
import type { ChangedFile } from '../api/gen/ChangedFile';

/** A diff with more changed lines than this starts hidden behind "Load diff". */
const LARGE_DIFF_LINES = 1000;

/** Why a file's diff starts hidden behind "Load diff" (as on GitHub), or null to show it. */
export function hiddenReason(f: ChangedFile): 'binary' | 'generated' | 'large' | null {
  if (f.binary) return 'binary';
  if (f.generated) return 'generated';
  if (f.added + f.removed > LARGE_DIFF_LINES) return 'large';
  return null;
}

/** About how tall a file's diff is before it has been drawn (changed lines, a little context), in px. */
export function estimatedHeight(f: ChangedFile): number {
  const lines = f.added + f.removed;
  return lines === 0 ? 48 : Math.min(lines, LARGE_DIFF_LINES) * 20 + 60;
}

/** How far outside the scrolling pane a diff is drawn, and how far it goes before it is dropped again (px). */
const NEAR = 1200;
const FAR = 5000;

/**
 * Whether `el` is near the visible part of `root` (a scrolling pane): true once it comes within NEAR px, false again
 * once it is more than FAR px away. Without IntersectionObserver (tests) everything is near.
 */
export function useNearViewport(el: RefObject<Element | null>, root: RefObject<Element | null>): boolean {
  const [near, setNear] = useState(() => typeof IntersectionObserver === 'undefined');
  useEffect(() => {
    const target = el.current;
    if (typeof IntersectionObserver === 'undefined' || !target) return;
    const opts = { root: root.current };
    const enter = new IntersectionObserver((es) => {
      if (es.some((e) => e.isIntersecting)) setNear(true);
    }, { ...opts, rootMargin: `${NEAR}px 0px` });
    const leave = new IntersectionObserver((es) => {
      if (es.every((e) => !e.isIntersecting)) setNear(false);
    }, { ...opts, rootMargin: `${FAR}px 0px` });
    enter.observe(target);
    leave.observe(target);
    return () => {
      enter.disconnect();
      leave.disconnect();
    };
  }, [el, root]);
  return near;
}
