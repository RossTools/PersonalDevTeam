// The visual viewport on phones (PDT-113): whether the on-screen keyboard is up, and the box it leaves visible.
import { useEffect, useState } from 'react';

/** How much shorter (px) than the layout viewport the visual one must be for an on-screen keyboard to be up. A hardware
 * keyboard leaves at most a shortcut bar (iOS's is about 55 px). */
const KEYBOARD_PX = 150;

/** Pinch zoom also shrinks the visual viewport; it isn't a keyboard. */
const zoomed = (vv: VisualViewport) => vv.scale > 1.01;

/**
 * Whether an on-screen keyboard covers part of the page: the visual viewport is much shorter than the layout one.
 * False without `visualViewport`, or with a hardware keyboard (which brings no on-screen one).
 */
export function softKeyboardOpen(): boolean {
  const vv = typeof window !== 'undefined' ? window.visualViewport : null;
  if (!vv || zoomed(vv)) return false;
  return document.documentElement.clientHeight - vv.height > KEYBOARD_PX;
}

/**
 * The part of the page the user can see (top and height, px) while an on-screen keyboard is up; null otherwise, or
 * when `on` is false. A layout pinned to it keeps its bottom (a composer) above the keyboard.
 */
export function useKeyboardBox(on: boolean): { top: number; height: number } | null {
  const [box, setBox] = useState<{ top: number; height: number } | null>(null);
  useEffect(() => {
    const vv = window.visualViewport;
    if (!on || !vv) {
      setBox(null);
      return;
    }
    const update = () => {
      const next = softKeyboardOpen() ? { top: Math.round(vv.offsetTop), height: Math.round(vv.height) } : null;
      setBox((b) => (b?.top === next?.top && b?.height === next?.height ? b : next));
    };
    update();
    vv.addEventListener('resize', update);
    vv.addEventListener('scroll', update);
    return () => {
      vv.removeEventListener('resize', update);
      vv.removeEventListener('scroll', update);
    };
  }, [on]);
  return box;
}
