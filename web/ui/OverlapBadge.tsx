import { cn } from '@ui/lib/utils';
import { Badge } from './Badge';

/** How a ticket overlaps others (the API's `OverlapLevel`). */
export type OverlapBadgeLevel = 'none' | 'overlap' | 'conflict';

export interface OverlapBadgeProps {
  level: OverlapBadgeLevel;
  /** The other tickets' keys, conflicts first: the first is named, the rest counted ("+2"). */
  keys: string[];
  className?: string;
}

/**
 * Shared work with another active ticket: amber "Overlaps PDT-51" (shared areas or files) or red "Conflicts with
 * PDT-51" (an exclusive area, or a file in one), "+N" for the rest. Renders nothing for `none`.
 */
export function OverlapBadge({ level, keys, className }: OverlapBadgeProps) {
  if (level === 'none') return null;
  const conflict = level === 'conflict';
  const verb = conflict ? 'Conflicts with' : 'Overlaps';
  const [first, ...rest] = keys;
  const text = first ? `${verb} ${first}` : conflict ? 'Conflicts' : 'Overlaps';
  const all = keys.length > 0 ? `${verb} ${keys.join(', ')}` : text;
  return (
    <Badge tone={conflict ? 'failed' : 'waiting'} title={all} className={cn('min-w-0', className)}>
      <span className="min-w-0 truncate">{text}</span>
      {rest.length > 0 && (
        <>
          <span aria-hidden className="tabular-nums">+{rest.length}</span>
          <span className="sr-only">{` and ${rest.length} more`}</span>
        </>
      )}
    </Badge>
  );
}
