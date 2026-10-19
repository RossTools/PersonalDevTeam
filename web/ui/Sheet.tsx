import { lazy, Suspense } from 'react';
import type { SheetProps } from './SheetImpl';

export type { SheetProps, SheetSide } from './SheetImpl';

// The sheet is built on Base UI's drawer (swipe to close, snap points), which is large and only phones use, so it
// loads the first time a sheet renders instead of with the app.
const load = () => import('./SheetImpl').then((m) => ({ default: m.Sheet }));
const Impl = lazy(load);
// Fetch it once the app is idle, so a phone's first sheet opens without a wait.
if (typeof window !== 'undefined') {
  const idle = (window as Window & { requestIdleCallback?: (cb: () => void) => void }).requestIdleCallback;
  if (idle) idle(() => void load()); else setTimeout(() => void load(), 2000);
}

/** A panel from the left, right or bottom edge; opened by a button or `open`, closed by a swipe, Escape, Close or (modal only) a tap on the backdrop. */
export function Sheet(props: SheetProps) {
  return (
    <Suspense fallback={props.trigger ?? null}>
      <Impl {...props} />
    </Suspense>
  );
}
