import { useLayoutEffect, useRef, useState } from 'react';
import { cn } from '@ui/lib/utils';
import type { PartState } from '../api/gen/PartState';
import type { PartsProgress } from '../api/gen/PartsProgress';
import { plural } from './text';

export type { PartState, PartsProgress };

/** Status colours (state only): done and merged in the accent, review waiting, in progress running. */
const SEGMENT: Record<PartState, string> = {
  done: 'bg-primary',
  merged: 'bg-primary opacity-60',
  review: 'bg-waiting',
  progress: 'bg-running',
  todo: 'bg-idle',
  backlog: 'bg-border',
};

/** `2/4`: closed parts of all parts. */
const partsCount = (p: Pick<PartsProgress, 'done' | 'total'>) => `${p.done}/${p.total}`;

/** "2 of 4 parts done, 1 in review, 1 in progress": what the stepper shows, for screen readers and tooltips. */
function partsSummary(p: PartsProgress): string {
  const rest = (
    [
      ['merged', p.merged],
      ['in review', p.review],
      ['in progress', p.progress],
      ['to do', p.todo],
      ['not started', p.backlog],
    ] as const
  )
    .filter(([, n]) => n > 0)
    .map(([what, n]) => `${n} ${what}`);
  return [`${p.done} of ${plural(p.total, 'part')} done`, ...rest].join(', ');
}

export interface PartsStepperProps {
  parts: PartsProgress;
  /** Adds the `2/4` count after the segments. */
  showCount?: boolean;
  className?: string;
}

/** Above this many parts the stepper is one bar of proportional segments: separate segments would be slivers. */
export const PARTS_BAR_ABOVE = 20;

/** The narrowest a part's dot gets (px), and the gap between dots: narrower than this, the stepper is a bar. */
const DOT_MIN_PX = 4;
const DOT_GAP_PX = 2;

/** Whether `n` dots fit in `width` px; an unmeasured width (0, as in tests) counts as fitting. */
export const dotsFit = (n: number, width: number) => width <= 0 || width >= n * DOT_MIN_PX + (n - 1) * DOT_GAP_PX;

/** The bar's segments, in this order: closed first, not started last. */
const BAR_ORDER: PartState[] = ['done', 'merged', 'review', 'progress', 'todo', 'backlog'];

/**
 * One dot per part, in order, coloured by where it is (done, merged, in review, in progress, to do, not started).
 * Past `PARTS_BAR_ABOVE` parts, or when the dots don't fit the width the stepper is given, one bar instead, with a
 * segment per state sized by its count. The tooltip gives the counts.
 */
export function PartsStepper({ parts, showCount = false, className }: PartsStepperProps) {
  const summary = partsSummary(parts);
  const track = useRef<HTMLSpanElement>(null);
  const [width, setWidth] = useState(0);
  useLayoutEffect(() => {
    const el = track.current;
    if (!el) return;
    setWidth(el.clientWidth);
    if (typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver(() => setWidth(el.clientWidth));
    observer.observe(el);
    return () => observer.disconnect();
  }, []);
  const n = parts.segments.length;
  const bar = n > PARTS_BAR_ABOVE || !dotsFit(n, width);
  return (
    <span role="img" aria-label={summary} title={summary} className={cn('flex min-w-0 items-center gap-1.5', className)}>
      {/* The track's width doesn't depend on what it holds, so measuring it never flips dots and bar back and forth. */}
      <span ref={track} aria-hidden className="flex min-w-8 flex-1 basis-0 items-center overflow-hidden">
        {bar ? (
          <span data-bar className="flex h-1.5 w-full overflow-hidden rounded-full bg-border">
            {BAR_ORDER.filter((s) => parts[s] > 0).map((s) => (
              <span key={s} data-state={s} className={cn('h-full min-w-0.5', SEGMENT[s])} style={{ flexGrow: parts[s], flexBasis: 0 }} />
            ))}
          </span>
        ) : (
          <span className="flex w-full items-center gap-0.5">
            {parts.segments.map((s, i) => (
              <span key={i} data-state={s} className={cn('h-1.5 min-w-1 flex-1 rounded-full', SEGMENT[s])} />
            ))}
          </span>
        )}
      </span>
      {showCount && <span aria-hidden className="shrink-0 text-meta text-muted-foreground tabular-nums">{partsCount(parts)}</span>}
    </span>
  );
}
