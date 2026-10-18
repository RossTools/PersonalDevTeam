import { useState } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import { cn } from '@ui/lib/utils';
import { Button, EmptyState, MenuButton, Skeleton, useConfirm, ViewHeader } from '@ui';
import type { Ticket } from '../api/gen/Ticket';
import { PriorityIcon } from '../common/PriorityIcon';
import { StageStepper } from '../common/StageStepper';
import { TicketKey } from '../common/TicketKey';
import { useAction } from '../common/useAction';
import { deleteTicket, moveToEntries, patchTicket } from './actions';
import { useTicket } from './queries';
import { LabelChips, TicketAreas, TicketDescription, TicketDetails, TicketLinks, TicketTimeline, TicketTitle } from './TicketParts';

type HubTab = 'overview' | 'conversation' | 'changes';

const HUB_TABS: { id: HubTab; label: string; arrives?: string }[] = [
  { id: 'overview', label: 'Overview' },
  { id: 'conversation', label: 'Conversation', arrives: 'Phase 5' },
  { id: 'changes', label: 'Changes', arrives: 'Phase 6' },
];

/** Overview · Conversation · Changes; the last two arrive with structured agents (5) and review (6). */
function HubTabs({ value }: { value: HubTab }) {
  return (
    <div role="tablist" aria-label="Ticket sections" className="flex shrink-0 items-center gap-1 border-b border-border px-3">
      {HUB_TABS.map((t) => (
        <button
          key={t.id}
          type="button"
          role="tab"
          aria-selected={t.id === value}
          aria-disabled={t.arrives ? true : undefined}
          disabled={!!t.arrives}
          title={t.arrives ? `Arrives in ${t.arrives}` : undefined}
          className={cn(
            '-mb-px border-b-2 px-2 py-1.5 text-xs outline-none focus-visible:ring-2 focus-visible:ring-ring pointer-coarse:min-h-control-touch',
            t.id === value ? 'border-primary font-medium text-foreground' : 'border-transparent text-muted-foreground',
            t.arrives && 'cursor-not-allowed opacity-60',
          )}
        >
          {t.label}
          {t.arrives && <span className="ml-1 text-2xs">({t.arrives})</span>}
        </button>
      ))}
    </div>
  );
}

export function TicketHeader({ ticket, onDeleted }: { ticket: Ticket; onDeleted?: () => void }) {
  const qc = useQueryClient();
  const confirm = useConfirm();
  return (
    <ViewHeader
      leading={<TicketKey ticketKey={ticket.key} />}
      title={<TicketTitle ticket={ticket} />}
      titleTooltip={ticket.title}
      meta={
        <>
          <StageStepper stage={ticket.stage} compact />
          <PriorityIcon priority={ticket.priority} />
          <LabelChips labels={ticket.labels} className="hidden sm:flex" />
        </>
      }
      actions={
        <MenuButton
          label="Ticket actions"
          entries={() => [
            { submenu: 'Move to', entries: moveToEntries(qc, ticket) },
            { separator: true },
            {
              label: 'Delete…',
              danger: true,
              onSelect: () => void deleteTicket(confirm, qc, ticket).then((ok) => { if (ok) onDeleted?.(); }),
            },
          ]}
        />
      }
    />
  );
}

/** A ticket tab: header, the section tabs, then the Overview (description and activity; details and links aside). */
export function TicketView({ ticketId, onDeleted }: { ticketId: string; onDeleted?: () => void }) {
  const q = useTicket(ticketId);
  const [tab] = useState<HubTab>('overview');
  if (q.isPending) return <div className="p-4"><Skeleton lines={3} label="Loading ticket" /></div>;
  if (!q.data) {
    return (
      <EmptyState action={<Button size="sm" onClick={() => void q.refetch()}>Retry</Button>}>
        {q.error?.message ?? 'Ticket not found.'}
      </EmptyState>
    );
  }
  const t = q.data;
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <TicketHeader ticket={t} onDeleted={onDeleted} />
      <HubTabs value={tab} />
      <div role="tabpanel" aria-label="Overview" className="min-h-0 flex-1 overflow-auto">
        <div className="mx-auto grid max-w-6xl gap-6 p-4 md:grid-cols-[minmax(0,1fr)_18rem]">
          <div className="flex min-w-0 flex-col divide-y divide-border *:py-5 *:first:pt-0">
            <TicketDescription key={t.id} ticket={t} />
            <TicketTimeline ticketId={t.id} composer />
          </div>
          <aside aria-label="Ticket details" className="flex min-w-0 flex-col self-start rounded-md border border-border bg-card px-3 divide-y divide-border *:py-3">
            <TicketDetails ticket={t} />
            <TicketLinks ticket={t} />
            <TicketAreas ticket={t} />
          </aside>
        </div>
      </div>
    </div>
  );
}
