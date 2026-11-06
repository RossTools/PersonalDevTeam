import { Chip } from '@ui/Chip';
import { cn } from '@ui/lib/utils';

/** A ticket's labels as chips, for headers, rows and cards. */
export function LabelChips({ labels, className }: { labels: string[]; className?: string }) {
  if (labels.length === 0) return null;
  return (
    <span className={cn('flex min-w-0 items-center gap-1', className)}>
      {labels.map((l) => <Chip key={l}>{l}</Chip>)}
    </span>
  );
}
