// Ticket keys and PR numbers in a conversation as pills (PDT-382): a ticket's opens a card about it on hover or focus.
import { useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { Badge, HoverCard, Skeleton } from '@ui';
import { cn } from '@ui/lib/utils';
import { api, ApiError } from '../api/client';
import type { Ticket } from '../api/gen/Ticket';
import { CI_LABEL, CI_TONE, prName } from '../tickets/PullRequest';
import { ticketKeys } from '../tickets/queries';
import { PRIORITY_LABEL } from './PriorityIcon';
import { STAGE_LABEL } from './StageStepper';
import { agentLine } from './ticketAgent';

/** A key or number in a pill: mono, a quiet fill and edge, the accent on hover; a bigger hit area on touch. */
const PILL =
  'relative touch-hit inline-flex items-center rounded-md border border-border bg-muted px-1 font-mono text-meta leading-[1.45] text-foreground no-underline outline-none align-baseline hover:border-primary hover:text-link focus-visible:ring-2 focus-visible:ring-ring';

/** The ticket a hover card shows, by key: fetched on the first hover, then kept a while (React Query). */
function useTicketCard(ticketKey: string, enabled: boolean) {
  return useQuery({
    queryKey: ticketKeys.card(ticketKey),
    queryFn: () => api.getTicket(ticketKey),
    enabled,
    staleTime: 30_000,
    retry: (n, e) => !(e instanceof ApiError && e.status === 404) && n < 2,
  });
}

/** What a ticket's hover card says: title, stage, priority, labels, who has it, the branch and the PR with its CI. */
function TicketCard({ ticket: t }: { ticket: Ticket }) {
  const agent = t.agent ? agentLine(t.agent) : null;
  const pr = t.attempt?.pr ?? null;
  const ci = pr?.ci ?? t.attempt?.ci ?? null;
  const active = t.attempt?.state === 'active';
  return (
    <>
      <div className="text-sm font-semibold">{t.title}</div>
      <div className="flex flex-wrap items-center gap-1">
        <Badge tone={t.stage === 'done' ? 'running' : 'neutral'}>{STAGE_LABEL[t.stage]}</Badge>
        {t.priority !== 'none' && <Badge>{PRIORITY_LABEL[t.priority]}</Badge>}
        {t.labels.map((l) => <Badge key={l} tone="accent">{l}</Badge>)}
      </div>
      {agent && <div className="text-muted-foreground">{agent.text}</div>}
      {active && t.attempt && (
        <div className="text-muted-foreground">
          Branch <span className="font-mono text-foreground">{t.attempt.branch}</span>
        </div>
      )}
      {(pr || ci) && (
        <div className="flex items-center gap-1.5 text-muted-foreground">
          {pr && <span className="text-foreground">{prName(pr)}</span>}
          {ci && <span className={cn(CI_TONE[ci.state] === 'failed' && 'text-failed', CI_TONE[ci.state] === 'running' && 'text-running')}>{CI_LABEL[ci.state]}</span>}
        </div>
      )}
    </>
  );
}

function CardBody({ ticketKey, open }: { ticketKey: string; open: boolean }) {
  const q = useTicketCard(ticketKey, open);
  if (q.data) return <TicketCard ticket={q.data} />;
  if (q.isError) {
    const gone = q.error instanceof ApiError && q.error.status === 404;
    return <div className="text-muted-foreground">{gone ? `No ticket ${ticketKey}` : `Couldn't load ${ticketKey}`}</div>;
  }
  return <Skeleton lines={2} label={`Loading ${ticketKey}`} className="w-56" />;
}

/** A ticket key as a pill that opens the ticket; hovering (or focusing) it shows its card. */
export function TicketPill({ ticketKey, className }: { ticketKey: string; className?: string }) {
  // Asked for on the first open, then kept: the card shows the cached ticket at once next time.
  const [asked, setAsked] = useState(false);
  return (
    <HoverCard onOpenChange={(o) => o && setAsked(true)} content={<CardBody ticketKey={ticketKey} open={asked} />}>
      <a href={`/tickets/${encodeURIComponent(ticketKey)}`} data-slot="ticket-pill" className={cn(PILL, className)}>
        {ticketKey}
      </a>
    </HoverCard>
  );
}

/** A pull request's number as a pill linking to it on the forge. */
export function PrPill({ number, href, className }: { number: number; href: string; className?: string }) {
  return (
    <a href={href} target="_blank" rel="noreferrer" data-slot="pr-pill" className={cn(PILL, className)}>
      PR #{number}
    </a>
  );
}
