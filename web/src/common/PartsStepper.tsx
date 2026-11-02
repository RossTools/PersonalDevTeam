import { cn } from '@ui/lib/utils';
import type { PartState } from '../api/gen/PartState';
import type { PartsProgress } from '../api/gen/PartsProgress';

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
export const partsCount = (p: Pick<PartsProgress, 'done' | 'total'>) => `${p.done}/${p.total}`;

/** "2 of 4 parts done, 1 in review, 1 in progress": what the stepper shows, for screen readers and tooltips. */
export function partsSummary(p: PartsProgress): string {
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
  return [`${p.done} of ${p.total} part${p.total === 1 ? '' : 's'} done`, ...rest].join(', ');
}

export interface PartsStepperProps {
  parts: PartsProgress;
  /** Adds the `2/4` count after the segments. */
  showCount?: boolean;
  className?: string;
}

/** One segment per part, in order, coloured by where it is (done, merged, in review, in progress, to do, not started). */
export function PartsStepper({ parts, showCount = false, className }: PartsStepperProps) {
  const summary = partsSummary(parts);
  return (
    <span role="img" aria-label={summary} title={summary} className={cn('flex min-w-0 items-center gap-1.5', className)}>
      <span aria-hidden className="flex min-w-8 flex-1 items-center gap-0.5">
        {parts.segments.map((s, i) => (
          <span key={i} data-state={s} className={cn('h-1.5 min-w-1.5 flex-1 rounded-full', SEGMENT[s])} />
        ))}
      </span>
      {showCount && <span aria-hidden className="shrink-0 text-meta text-muted-foreground tabular-nums">{partsCount(parts)}</span>}
    </span>
  );
}
