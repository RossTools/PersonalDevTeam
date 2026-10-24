import { useEffect, useState } from 'react';
import { CircleCheck, Clock, Timer } from 'lucide-react';
import { cn } from '@ui/lib/utils';
import { Badge, OverlapBadge } from '@ui';
import type { TicketSummary } from '../api/gen/TicketSummary';

type BadgeTicket = Pick<TicketSummary, 'blockedBy' | 'overlap' | 'overlapWith' | 'waiting' | 'clearToStart'> &
  Partial<Pick<TicketSummary, 'pipelineStage' | 'autoStartAt'>>;

/** `m:ss` until `at`, never below 0:00. */
export function countdown(at: number, now: number): string {
  const s = Math.max(0, Math.ceil((at - now) / 1000));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`;
}

/** "Auto-starts in 1:30" while the ticket waits out its project's start delay (PDT-146), ticking every second. */
function AutoStartBadge({ at }: { at: number }) {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const t = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(t);
  }, []);
  return (
    <Badge title="Auto-start takes it once it has gone the project's start delay unchanged; any change starts the wait again">
      <Timer aria-hidden /> Auto-starts in {countdown(at, now)}
    </Badge>
  );
}

/**
 * Refining, Merge blocked, Blocked, overlap/conflict, Waiting (or Clear to start) and Auto-starts in for rows, cards
 * and the ticket header.
 */
export function TicketBadges({ ticket: t, className }: { ticket: BadgeTicket; className?: string }) {
  const refining = t.pipelineStage === 'refine';
  const mergeBlocked = t.pipelineStage === 'merge_blocked';
  const autoStartAt = t.autoStartAt ?? null;
  const any = refining || mergeBlocked || t.blockedBy > 0 || t.overlap !== 'none' || t.waiting || autoStartAt !== null;
  if (!any) return null;
  return (
    <span className={cn('flex min-w-0 shrink items-center gap-1', className)}>
      {refining && <Badge title="The pipeline is refining it">Refining</Badge>}
      {mergeBlocked && <Badge tone="failed" title="Its merge was refused; the Inbox asks you">Merge blocked</Badge>}
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
      {autoStartAt !== null && <AutoStartBadge at={autoStartAt} />}
    </span>
  );
}
