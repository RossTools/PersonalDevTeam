import type { ReactNode } from 'react';
import { cn } from './lib/utils';

/**
 * A decision fixed under the content it is about: an optional note box, then the buttons (the primary one first,
 * with its key), a `hint` beside them, and `end` (Dismiss, Stop, Won't do) pushed to the right. `below` is for what
 * opens from a button (Take over's outcomes). The hint is a live region that is always there, so a screen reader says
 * each new hint ("Press again to confirm").
 */
export function ActionBar({ note, children, hint, end, below, className, 'aria-label': ariaLabel = 'Actions' }: {
  note?: ReactNode;
  children?: ReactNode;
  hint?: ReactNode;
  end?: ReactNode;
  below?: ReactNode;
  className?: string;
  'aria-label'?: string;
}) {
  return (
    <div role="group" aria-label={ariaLabel} className={cn('shrink-0 border-t border-border bg-card px-gutter py-3', className)}>
      <div className="flex max-w-3xl flex-col gap-2.5">
        {note}
        <div className="flex flex-wrap items-center gap-2">
          {children}
          <span aria-live="polite" className={cn('text-xs text-muted-foreground', !hint && 'sr-only')}>{hint}</span>
          {end && <span className="ml-auto flex flex-wrap items-center gap-2">{end}</span>}
        </div>
        {below}
      </div>
    </div>
  );
}
