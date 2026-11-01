import { useEffect, useState } from 'react';
import { CircleCheck, Clock, Timer } from 'lucide-react';
import { cn } from '@ui/lib/utils';
import { Badge, OverlapBadge } from '@ui';
import type { AgentBy } from '../api/gen/AgentBy';
import type { TicketSummary } from '../api/gen/TicketSummary';

type BadgeTicket = Pick<TicketSummary, 'blockedBy' | 'overlap' | 'overlapWith' | 'waiting' | 'clearToStart'> &
  Partial<Pick<TicketSummary, 'pipelineStage' | 'autoStartAt' | 'source' | 'admitted' | 'pickupHold' | 'createdBy'>>;

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

/** "Created by Coder · from PDT-345": the agent session that made a ticket and the ticket it was working on. */
export function createdByText(by: AgentBy | null | undefined, fallback: string): string {
  const who = by?.persona ?? fallback;
  return `Created by ${who}${by?.fromTicket ? ` · from ${by.fromTicket}` : ''}`;
}

/**
 * Who made a ticket, when not the user (PDT-201): an agent by its persona (PDT-363). An external one the user hasn't
 * let in yet is held back.
 */
function SourceBadge({ source, admitted, createdBy }: Pick<TicketSummary, 'source' | 'admitted'> & { createdBy?: AgentBy | null }) {
  if (source === 'pm') return <Badge title={createdByText(createdBy, 'the PM')}>PM</Badge>;
  if (source === 'agent') return <Badge title={createdByText(createdBy, 'an agent')}>{createdBy?.persona ?? 'Agent'}</Badge>;
  if (source === 'workflow') return <Badge title="A workflow made it">Workflow</Badge>;
  if (source !== 'external') return null;
  return admitted ? (
    <Badge title="Made from outside PDT (an issue or a report); you let it in">External</Badge>
  ) : (
    <Badge tone="waiting" title="Made from outside PDT (an issue or a report): only you can move it to To do, start it or label it agentic">
      External
    </Badge>
  );
}

/**
 * Who made it (unless you did), Refining, Merge blocked, Blocked, overlap/conflict, Waiting (or Clear to start), Auto-starts in and
 * why auto-start passed it over (PDT-329) for rows, cards
 * and the ticket header.
 */
export function TicketBadges({ ticket: t, className }: { ticket: BadgeTicket; className?: string }) {
  const refining = t.pipelineStage === 'refine';
  const mergeBlocked = t.pipelineStage === 'merge_blocked';
  const autoStartAt = t.autoStartAt ?? null;
  const source = t.source ?? 'user';
  const hold = t.pickupHold ?? null;
  const any =
    source !== 'user' || refining || mergeBlocked || t.blockedBy > 0 || t.overlap !== 'none' || t.waiting || autoStartAt !== null || hold !== null;
  if (!any) return null;
  return (
    <span className={cn('flex min-w-0 shrink items-center gap-1', className)}>
      <SourceBadge source={source} admitted={t.admitted ?? false} createdBy={t.createdBy ?? null} />
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
      {hold !== null && (
        <Badge tone="waiting" className="min-w-0" title={`Auto-start passed it over: ${hold}. It starts once that work is done.`}>
          <Clock aria-hidden /> <span className="truncate">Waiting: {hold}</span>
        </Badge>
      )}
    </span>
  );
}
