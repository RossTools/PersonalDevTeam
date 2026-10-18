import { cn } from '@ui/lib/utils';
import { Skeleton as SkeletonPrimitive } from '@ui/primitives/skeleton';

export interface SkeletonProps {
  /** Text placeholder: this many line-height bars, the last one shorter. Without it, one block sized by className. */
  lines?: number;
  /** Accessible name of the placeholder (it is a `role="status"`). */
  label?: string;
  className?: string;
}

/** Loading placeholder for lists (`lines`) and pages (a block: size it with `className`, default 4rem tall). */
export function Skeleton({ lines, label = 'Loading', className }: SkeletonProps) {
  if (lines === undefined) {
    return (
      <div role="status" aria-label={label} className={cn('h-16 w-full', className)}>
        <SkeletonPrimitive className="size-full" />
      </div>
    );
  }
  return (
    <div role="status" aria-label={label} className={cn('flex w-full flex-col gap-2', className)}>
      {Array.from({ length: lines }, (_, i) => (
        <SkeletonPrimitive key={i} data-testid="skeleton-line" className={cn('h-3', i === lines - 1 && lines > 1 ? 'w-3/5' : 'w-full')} />
      ))}
    </div>
  );
}
