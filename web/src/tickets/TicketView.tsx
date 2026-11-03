import { useState, type ReactNode } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import { Play } from 'lucide-react';
import { Banner, Button, DiffStat, EmptyState, MenuButton, Skeleton, TabPanel, Tabs, useConfirm, ViewHeader } from '@ui';
import type { Ticket } from '../api/gen/Ticket';
import { BranchChip } from '../common/BranchChip';
import { PriorityIcon } from '../common/PriorityIcon';
import { StageStepper } from '../common/StageStepper';
import { TicketKey } from '../common/TicketKey';
import { WatchButton } from '../common/Watch';
import { useAction } from '../common/useAction';
import { useRememberedScroll } from '../common/viewState';
import { AttemptSection, MergedBanner } from './Attempt';
import { canStart, retryAutoStart, ticketMenu, useStartTicket } from './actions';
import { canMakeParts, MakePartsDialog, ParentChip, PartsSection } from './PartsSection';
import { PartsStepper } from '../common/PartsStepper';
import { useClearToStartToasts } from './clearToStart';
import { TicketBadges } from './TicketBadges';
import { TicketConversation } from './TicketConversation';
import { ChangesView } from '../review/ChangesView';
import { MergeDialog } from '../review/MergeDialog';
import { useOpenChanges } from '../common/open';
import { useDiffSize } from '../review/queries';
import { useCheckMerge, useProjectForge, useTicket } from './queries';
import { CreateIssueDialog } from './CreateIssueDialog';
import { TicketAttachments } from './attachments';
import { TicketDocuments } from './TicketDocuments';
import { ExternalKeys } from './ExternalKeys';
import {
  LabelChips, TicketDescription, TicketDetails, TicketLinks, TicketOverlaps, TicketTimeline, TicketTitle, TicketWatchers,
} from './TicketParts';

export type HubTab = 'overview' | 'conversation' | 'changes';

const HUB_TABS: { id: HubTab; label: string }[] = [
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

/**
 * Overview · Conversation · Changes (with the branch's diff size), and what each shows: `overview` is the page's own
 * `<TabPanel id="overview">`.
 */
export function HubTabs({ ticket: t, value, onChange, diff, overview }: {
  ticket: Ticket;
  value: HubTab;
  onChange: (t: HubTab) => void;
  diff?: { added: number; removed: number; files: number } | null;
  overview: ReactNode;
}) {
  const tabs = HUB_TABS.map((h) => h.id === 'changes' && diff && diff.files > 0
    ? { ...h, label: <>{h.label}<DiffStat added={diff.added} removed={diff.removed} className="ml-1.5 text-2xs" /></> }
    : h);
  return (
    <Tabs aria-label="Ticket sections" tabs={tabs} value={value} onChange={onChange} listClassName="px-gutter">
      {overview}
      <TabPanel id="conversation"><TicketConversation ticket={t} /></TabPanel>
      <TabPanel id="changes"><TicketChanges ticket={t} /></TabPanel>
    </Tabs>
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
      gutter
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
          <WatchButton of={{ ticket: ticket.id }} />
          {canStart(ticket) && (
            <Button variant="primary" size="sm" onClick={() => start(ticket, { here: true })}>
              <Play aria-hidden /> Start
            </Button>
          )}
          <MenuButton
            label="Ticket actions"
            entries={() => ticketMenu({
              qc, confirm, ticket, startPart: start, abandon: true,
              ...(canMakeParts(ticket) ? { onMakeParts: () => setMaking(true) } : {}),
              onMerge: () => setMerging(true),
              ...(canFile ? { onFileIssue: () => setFiling(true) } : {}),
              ...(onDeleted ? { onDeleted } : {}),
            })}
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

/** Auto-start passes the ticket over because its start failed: the error, and Try again. */
export function AutoStartFailedBanner({ ticket: t }: { ticket: Ticket }) {
  const qc = useQueryClient();
  const [retry, pending] = useAction(() => retryAutoStart(qc, t));
  if (!t.pickupFailed) return null;
  const text = `Auto-start couldn't start it: ${t.pickupFailed}`;
  return (
    <Banner
      tone="failed"
      title={`${text}. It tries again once the ticket changes.`}
      actions={
        <Button size="sm" compact pending={pending} onClick={() => void retry()}>
          Try again
        </Button>
      }
    >
      {text}
    </Banner>
  );
}

/** The Changes: the attempt's diff and review. */
function TicketChanges({ ticket: t }: { ticket: Ticket }) {
  return t.attempt?.worktreeId ? (
    <ChangesView key={t.attempt.worktreeId} worktreeId={t.attempt.worktreeId} ticket={t} />
  ) : (
    <EmptyState>{t.attempt ? 'The worktree of this attempt is gone.' : 'Start the ticket to see its changes here.'}</EmptyState>
  );
}

/** A ticket tab: header, the section tabs, then the Overview (description and activity; details and links aside) or
 * the Conversation (its agent sessions) or the Changes (its attempt's diff and review). */
/** `mergedBanner` false: the shell already shows it above the ticket's workspace. */
export function TicketView({ ticketId, onDeleted, mergedBanner = true }: { ticketId: string; onDeleted?: () => void; mergedBanner?: boolean }) {
  const q = useTicket(ticketId);
  const [tab, setTab] = useSection(ticketId);
  // The Overview's scroll comes back when the ticket is left and opened again (PDT-379).
  const overview = useRememberedScroll(`ticket:${ticketId}:overview`);
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
      <AutoStartFailedBanner ticket={t} />
      <HubTabs
        ticket={t}
        value={tab}
        onChange={setTab}
        diff={diff}
        overview={
          <TabPanel id="overview" ref={overview.ref} onScroll={overview.onScroll} className="block overflow-auto">
            <div className="mx-auto grid max-w-6xl gap-6 p-gutter md:grid-cols-[minmax(0,1fr)_18rem]">
              <div className="flex min-w-0 flex-col divide-y divide-border *:py-5 *:first:pt-0">
                <TicketDescription key={t.id} ticket={t} />
                <PartsSection ticket={t} />
                <TicketTimeline ticketId={t.id} composer />
              </div>
              <aside aria-label="Ticket details" className="flex min-w-0 flex-col self-start rounded-md border border-border bg-card px-3 divide-y divide-border *:py-3">
                <TicketDetails ticket={t} />
                <TicketWatchers ticketId={t.id} />
                <AttemptSection ticket={t} />
                <TicketOverlaps ticket={t} />
                <TicketLinks ticket={t} />
                <TicketDocuments ticket={t} />
                <TicketAttachments ticketId={t.id} />
              </aside>
            </div>
          </TabPanel>
        }
      />
    </div>
  );
}
