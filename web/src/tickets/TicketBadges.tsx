import { CircleCheck, Clock } from 'lucide-react';
import { cn } from '@ui/lib/utils';
import { Badge, OverlapBadge } from '@ui';
import type { TicketSummary } from '../api/gen/TicketSummary';

type BadgeTicket = Pick<TicketSummary, 'blockedBy' | 'overlap' | 'overlapWith' | 'waiting' | 'clearToStart'>;

/** Blocked, overlap/conflict, and Waiting (or Clear to start) for rows, cards and the ticket header. */
export function TicketBadges({ ticket: t, className }: { ticket: BadgeTicket; className?: string }) {
  const any = t.blockedBy > 0 || t.overlap !== 'none' || t.waiting;
  if (!any) return null;
  return (
    <span className={cn('flex min-w-0 shrink items-center gap-1', className)}>
      {t.blockedBy > 0 && <Badge tone="waiting" title={`${t.blockedBy} open blocker${t.blockedBy === 1 ? '' : 's'}`}>Blocked</Badge>}
      <OverlapBadge level={t.overlap} keys={t.overlapWith} />
      {t.waiting && (t.clearToStart ? (
        <Badge tone="running" title="Nothing is in the way any more">
          <CircleCheck aria-hidden /> Clear to start
        </Badge>
      ) : (
        <Badge tone="waiting" title="Waiting until clear">
          <Clock aria-hidden /> Waiting
        </Badge>
      ))}
    </span>
  );
}
