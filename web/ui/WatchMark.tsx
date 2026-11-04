import { Eye } from 'lucide-react';
import { cn } from '@ui/lib/utils';

export interface WatchMarkProps {
  /** Who or how many watch, in words ("Watched by you and PM “Docs”"): its accessible name and hover text. */
  title: string;
  /** How many, shown beside the eye. */
  count?: number;
  className?: string;
}

/** A quiet eye: something watches this (a ticket on a card, a conversation in a list). Says who on hover. */
export function WatchMark({ title, count, className }: WatchMarkProps) {
  return (
    <span
      role="img"
      aria-label={title}
      title={title}
      data-slot="watch-mark"
      className={cn('inline-flex shrink-0 items-center gap-0.5 text-meta text-muted-foreground [&>svg]:size-3', className)}
    >
      <Eye aria-hidden />
      {count !== undefined && <span aria-hidden className="tabular-nums">{count}</span>}
    </span>
  );
}
