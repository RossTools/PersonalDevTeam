import { useState } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import { Play } from 'lucide-react';
import { cn } from '@ui/lib/utils';
import { Button, DiffStat, EmptyState, MenuButton, Skeleton, useConfirm, ViewHeader } from '@ui';
import type { Ticket } from '../api/gen/Ticket';
import { BranchChip } from '../common/BranchChip';
import { PriorityIcon } from '../common/PriorityIcon';
import { StageStepper } from '../common/StageStepper';
import { TicketKey } from '../common/TicketKey';
import { runAction } from '../common/useAction';
import { AttemptSection, MergedBanner } from './Attempt';
import {
  abandonAttempt, canRefine, canStart, deleteTicket, moveToEntries, refineTicket, startNextPart, stopWaitingEntry, useStartTicket,
} from './actions';
import { isClosed } from './model';
import { canMakeParts, MakePartsDialog, ParentChip, PartsSection } from './PartsSection';
import { PartsStepper } from '../common/PartsStepper';
import { useClearToStartToasts } from './clearToStart';
import { TicketBadges } from './TicketBadges';
import { TicketConversation } from './TicketConversation';
import { ChangesView } from '../review/ChangesView';
import { MergeDialog } from '../review/MergeDialog';
import { useOpenChanges } from '../review/open';
import { useDiffSize } from '../review/queries';
import { useCheckMerge, useProjectForge, useTicket } from './queries';
import { CreateIssueDialog } from './CreateIssueDialog';
import { TicketAttachments } from './attachments';
import { ExternalKeys } from './ExternalKeys';
import {
  LabelChips, TicketDescription, TicketDetails, TicketLinks, TicketOverlaps, TicketTimeline, TicketTitle,
} from './TicketParts';

export type HubTab = 'overview' | 'conversation' | 'changes';

const HUB_TABS: { id: HubTab; label: string; arrives?: string }[] = [
  { id: 'overview', label: 'Overview' },
  { id: 'conversation', label: 'Conversation' },
  { id: 'changes', label: 'Changes' },
];

const SECTION_KEY = (ticketId: string) => `pdt.ticket.section.${ticketId}`;

/** The section last shown for a ticket on this browser (Overview the first time). */
export function useSection(ticketId: string): [HubTab, (t: HubTab) => void] {
  const [tab, setTab] = useState<HubTab>(() => {
    try {
      const v = localStorage.getItem(SECTION_KEY(ticketId));
      return HUB_TABS.some((t) => t.id === v) ? (v as HubTab) : 'overview';
    } catch {
      return 'overview';
    }
  });
  const set = (t: HubTab) => {
    setTab(t);
    try {
      localStorage.setItem(SECTION_KEY(ticketId), t);
    } catch {
      // Storage blocked: remembered for this page only.
    }
  };
  return [tab, set];
}

/** Overview · Conversation · Changes (with the branch's diff size). */
export function HubTabs({ value, onChange, diff }: {
  value: HubTab;
  onChange: (t: HubTab) => void;
  diff?: { added: number; removed: number; files: number } | null;
}) {
  return (
    <div role="tablist" aria-label="Ticket sections" className="flex shrink-0 items-center gap-1 border-b border-border px-3">
      {HUB_TABS.map((t) => (
        <button
          key={t.id}
          type="button"
          role="tab"
          id={`ticket-tab-${t.id}`}
          aria-selected={t.id === value}
          onClick={() => !t.arrives && onChange(t.id)}
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
          {t.id === 'changes' && diff && diff.files > 0 && <DiffStat added={diff.added} removed={diff.removed} className="ml-1.5 text-2xs" />}
        </button>
      ))}
    </div>
  );
}

export function TicketHeader({ ticket, onDeleted }: { ticket: Ticket; onDeleted?: () => void }) {
  const qc = useQueryClient();
  const confirm = useConfirm();
  const start = useStartTicket();
  const attempt = ticket.attempt;
  const [making, setMaking] = useState(false);
  const [merging, setMerging] = useState(false);
  const [filing, setFiling] = useState(false);
  const forge = useProjectForge(ticket.projectId).data;
  // A ticket links one GitHub issue (PDT-221).
  const canFile = !!forge?.issues && !ticket.external.some((l) => l.system === 'github');
  const openChanges = useOpenChanges();
  return (
    <>
    <ViewHeader
      leading={
        <span className="flex items-center gap-1">
          {ticket.parent && <ParentChip parent={ticket.parent} projectId={ticket.projectId} className="hidden sm:inline-flex" />}
          <TicketKey ticketKey={ticket.key} />
          <ExternalKeys links={ticket.external} linked />
        </span>
      }
      title={<TicketTitle ticket={ticket} />}
      titleTooltip={ticket.title}
      meta={
        <>
          <StageStepper stage={ticket.stage} compact />
          <PriorityIcon priority={ticket.priority} />
          <TicketBadges ticket={ticket} />
          {ticket.parts && <PartsStepper parts={ticket.parts} showCount className="hidden w-28 sm:flex" />}
          <LabelChips labels={ticket.labels} className="hidden sm:flex" />
          {attempt && (
            <BranchChip
              branch={attempt.branch}
              base={attempt.baseBranch}
              {...(attempt.worktreeId ? { onReview: () => openChanges({ id: attempt.worktreeId!, projectId: ticket.projectId }) } : {})}
              className="hidden max-w-60 md:inline-flex"
            />
          )}
        </>
      }
      actions={
        <>
          {canStart(ticket) && (
            <Button variant="primary" size="sm" onClick={() => start(ticket, { here: true })}>
              <Play aria-hidden /> Start
            </Button>
          )}
          <MenuButton
            label="Ticket actions"
            entries={() => [
              ...(canRefine(ticket) ? [{ label: 'Refine', onSelect: () => void runAction(() => refineTicket(qc, ticket)) }] : []),
              ...(ticket.waiting ? [stopWaitingEntry(qc, ticket)] : []),
              { submenu: 'Move to', entries: moveToEntries(qc, ticket, confirm) },
              ...(ticket.parts && !isClosed(ticket.stage)
                ? [{ label: 'Start next part', onSelect: () => void runAction(() => startNextPart(start, ticket)) }]
                : []),
              ...(canMakeParts(ticket) ? [{ label: 'Make into parts…', onSelect: () => setMaking(true) }] : []),
              ...(attempt?.state === 'active' ? [{ label: 'Merge…', onSelect: () => setMerging(true) }] : []),
              ...(canFile ? [{ label: 'Create GitHub issue…', onSelect: () => setFiling(true) }] : []),
              ...(attempt?.state === 'active'
                ? [{ label: 'Abandon attempt…', onSelect: () => void runAction(() => abandonAttempt(confirm, qc, ticket, attempt)) }]
                : []),
              { separator: true },
              {
                label: 'Delete…',
                danger: true,
                onSelect: () => void runAction(async () => { if (await deleteTicket(confirm, qc, ticket)) onDeleted?.(); }),
              },
            ]}
          />
        </>
      }
    />
    {making && <MakePartsDialog ticket={ticket} onClose={() => setMaking(false)} />}
    {merging && attempt && <MergeDialog ticket={ticket} attemptId={attempt.id} onClose={() => setMerging(false)} />}
    {filing && forge && <CreateIssueDialog projectId={ticket.projectId} repo={forge.repo} ticket={ticket} onClose={() => setFiling(false)} />}
    </>
  );
}

/** The Conversation (the ticket's agent sessions) or the Changes (its attempt's diff and review). */
export function HubPanel({ ticket: t, tab }: { ticket: Ticket; tab: 'conversation' | 'changes' }) {
  if (tab === 'conversation') {
    return (
      <div role="tabpanel" aria-labelledby="ticket-tab-conversation" className="flex min-h-0 flex-1 flex-col">
        <TicketConversation ticket={t} />
      </div>
    );
  }
  return (
    <div role="tabpanel" aria-labelledby="ticket-tab-changes" className="flex min-h-0 flex-1 flex-col">
      {t.attempt?.worktreeId ? (
        <ChangesView key={t.attempt.worktreeId} worktreeId={t.attempt.worktreeId} ticket={t} />
      ) : (
        <EmptyState>{t.attempt ? 'The worktree of this attempt is gone.' : 'Start the ticket to see its changes here.'}</EmptyState>
      )}
    </div>
  );
}

/** A ticket tab: header, the section tabs, then the Overview (description and activity; details and links aside) or
 * the Conversation (its agent sessions) or the Changes (its attempt's diff and review). */
/** `mergedBanner` false: the shell already shows it above the ticket's workspace. */
export function TicketView({ ticketId, onDeleted, mergedBanner = true }: { ticketId: string; onDeleted?: () => void; mergedBanner?: boolean }) {
  const q = useTicket(ticketId);
  const [tab, setTab] = useSection(ticketId);
  const diff = useDiffSize(q.data?.attempt?.state === 'active' ? q.data.attempt.worktreeId : null);
  useCheckMerge(q.data);
  useClearToStartToasts(q.data ? [q.data] : []);
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
      {mergedBanner && <MergedBanner ticket={t} />}
      <HubTabs value={tab} onChange={setTab} diff={diff} />
      {tab !== 'overview' ? (
        <HubPanel ticket={t} tab={tab} />
      ) : (
        <div role="tabpanel" aria-labelledby="ticket-tab-overview" className="min-h-0 flex-1 overflow-auto">
          <div className="mx-auto grid max-w-6xl gap-6 p-4 md:grid-cols-[minmax(0,1fr)_18rem]">
            <div className="flex min-w-0 flex-col divide-y divide-border *:py-5 *:first:pt-0">
              <TicketDescription key={t.id} ticket={t} />
              <PartsSection ticket={t} />
              <TicketTimeline ticketId={t.id} composer />
            </div>
            <aside aria-label="Ticket details" className="flex min-w-0 flex-col self-start rounded-md border border-border bg-card px-3 divide-y divide-border *:py-3">
              <TicketDetails ticket={t} />
              <AttemptSection ticket={t} />
              <TicketOverlaps ticket={t} />
              <TicketLinks ticket={t} />
              <TicketAttachments ticketId={t.id} />
            </aside>
          </div>
        </div>
      )}
    </div>
  );
}
