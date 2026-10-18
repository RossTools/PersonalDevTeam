import { Minus, OctagonAlert } from 'lucide-react';
import { cn } from '@ui/lib/utils';
import type { Priority } from '../api/gen/Priority';

export type { Priority };

export const PRIORITY_LABEL: Record<Priority, string> = {
  none: 'No priority',
  low: 'Low',
  medium: 'Medium',
  high: 'High',
  urgent: 'Urgent',
};

/** Highest first, for sorting and menus. */
export const PRIORITIES: Priority[] = ['urgent', 'high', 'medium', 'low', 'none'];

const LEVEL: Partial<Record<Priority, number>> = { low: 1, medium: 2, high: 3 };

/** Three rising bars, the first `level` solid and the rest faint (lucide's signal icons are too small at 14px). */
function Bars({ level }: { level: number }) {
  return (
    <svg aria-hidden viewBox="0 0 14 14" className="size-3.5 shrink-0" fill="currentColor">
      {[0, 1, 2].map((i) => (
        <rect key={i} x={1.5 + i * 4} y={9 - i * 3.5} width={3} height={3.5 + i * 3.5} rx={0.75} opacity={i < level ? 1 : 0.25} />
      ))}
    </svg>
  );
}

export interface PriorityIconProps {
  priority: Priority;
  /** Show the label beside the icon (otherwise it is for screen readers only). */
  showLabel?: boolean;
  className?: string;
}

/**
 * A priority as an icon (bars for low → high, a dash for none). Urgent is the one that asks for attention, so it
 * takes the `waiting` state colour; the rest stay neutral (colour means state, spec §3).
 */
export function PriorityIcon({ priority, showLabel = false, className }: PriorityIconProps) {
  const level = LEVEL[priority];
  const label = PRIORITY_LABEL[priority];
  return (
    <span
      data-priority={priority}
      title={showLabel ? undefined : label}
      className={cn(
        'inline-flex shrink-0 items-center gap-1 text-xs',
        priority === 'urgent' ? 'text-waiting' : priority === 'none' ? 'text-muted-foreground' : 'text-foreground',
        className,
      )}
    >
      {priority === 'urgent' ? (
        <OctagonAlert aria-hidden className="size-3.5 shrink-0" />
      ) : level ? (
        <Bars level={level} />
      ) : (
        <Minus aria-hidden className="size-3.5 shrink-0" />
      )}
      <span className={showLabel ? undefined : 'sr-only'}>{label}</span>
    </span>
  );
}
