import type { ReactNode } from 'react';
import { cn } from './lib/utils';

export interface Fact {
  /** The figure: "9 GB". */
  value: ReactNode;
  /** What it counts: "free of 994 GB". */
  label: ReactNode;
}

/** A few figures side by side, each a big value over what it counts (free space, what cleaning would free). */
export function Facts({ facts, className, 'aria-label': ariaLabel }: { facts: Fact[]; className?: string; 'aria-label'?: string }) {
  return (
    <dl aria-label={ariaLabel} className={cn('grid grid-cols-[repeat(auto-fit,minmax(9rem,1fr))] gap-2', className)}>
      {facts.map((f, n) => (
        <div key={n} className="flex flex-col-reverse rounded-lg border border-border bg-card px-3 py-2">
          <dt className="text-xs text-muted-foreground">{f.label}</dt>
          <dd className="text-base font-semibold tabular-nums">{f.value}</dd>
        </div>
      ))}
    </dl>
  );
}
