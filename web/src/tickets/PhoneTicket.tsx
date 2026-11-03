// A ticket on a phone (PDT-119): a full screen in place of the peek. The header reads its state at a
// glance; the Overview stacks into one column with the details folded at the top; the bottom bar holds Comment… and
// the stage's primary action, everything else is under the top bar's ⋯.
import { useState, type ReactNode } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import { ChevronDown, ChevronRight, Pencil } from 'lucide-react';
import { Alert, Button, IconButton, Sheet, Textarea, useConfirm, type MenuEntry } from '@ui';
import { api } from '../api/client';
import type { InboxItem } from '../api/gen/InboxItem';
import type { Ticket } from '../api/gen/Ticket';
import { AgentDot } from '../common/AgentState';
import { Markdown } from '../common/Markdown';
import { useWatches, watchEntry, Watchers } from '../common/Watch';
import { PartsStepper } from '../common/PartsStepper';
import { PRIORITY_LABEL, PriorityIcon } from '../common/PriorityIcon';
import { STAGE_LABEL, StageStepper } from '../common/StageStepper';
import { actionError, runAction } from '../common/useAction';
import { MergeConfirm } from '../inbox/PhoneInbox';
import { useAnswer } from '../inbox/useAnswer';
import { useInbox } from '../inbox/useInbox';
import { MergeDialog } from '../review/MergeDialog';
import { useDiffSize } from '../review/queries';
import { useTopBarActions, useTopBarEdit } from '../shell/PhoneTopBar';
import {
  abandonAttempt, canRefine, canStart, deleteTicket, moveToEntries, patchTicket, priorityEntries, refineTicket, retryAutoStartEntry,
  startNextPart, stopWaitingEntry, useOpenSession, useOpenTicket, useStartTicket,
} from './actions';
import { AttemptSection, MergedBanner } from './Attempt';
import { agentLine } from './Board';
import { isClosed } from './model';
import { canMakeParts, MakePartsDialog, ParentChip, PartsSection } from './PartsSection';
import { prName } from './PullRequest';
import { TicketBadges } from './TicketBadges';
import { CommentComposer, FoldedSectionContext, OverviewSection, TicketDetails, TicketLinks, TicketOverlaps, TicketTimeline } from './TicketParts';
import { TicketAttachments, useTicketAttachments } from './attachments';
import { TicketDocuments } from './TicketDocuments';
import { useDocuments } from '../documents/queries';
import { HubPanel, HubTabs, useSection } from './TicketView';

/** The bottom bar's buttons: 48px, whatever the pointer. */
const big = 'h-12 pointer-coarse:h-12 min-w-0 flex-1';

/** What the bottom bar offers beside Comment…, by stage. */
export type PhoneTicketAction = 'refine' | 'start' | 'open' | 'accept';

/**
 * The stage's primary action(s): Draft/Ready → Refine · Start; To do → Start; In progress → Accept… while its Accept
 * gate waits in the Inbox, else Open work (with an active attempt); Done and Won't do → none.
 */
export function phoneTicketActions(t: Ticket, acceptOpen: boolean): PhoneTicketAction[] {
  if (isClosed(t.stage)) return [];
  if (t.stage === 'in_progress') {
    if (acceptOpen) return ['accept'];
    return t.attempt?.state === 'active' ? ['open'] : [];
  }
  return [...(canRefine(t) ? ['refine' as const] : []), ...(canStart(t) ? ['start' as const] : [])];
}

/** The ticket's Accept gate in the Inbox (under Ready to accept), if one waits. */
export function acceptItem(items: InboxItem[], ticketId: string): InboxItem | null {
  return items.find((i) => i.ticketId === ticketId && i.group === 'accept' && i.choices.some((c) => c.primary)) ?? null;
}

/** The field being edited, with its draft: Cancel and Save are in the top bar meanwhile. */
type Editing = { field: 'title' | 'description'; draft: string };

export function PhoneTicket({ ticket: t, onDeleted }: { ticket: Ticket; onDeleted: () => void }) {
  const qc = useQueryClient();
  const attachments = useTicketAttachments(t.id).data ?? [];
  const documents = useDocuments({ ticket: t.id }).data ?? [];
  const confirm = useConfirm();
  const start = useStartTicket();
  const openTicket = useOpenTicket();
  const openSession = useOpenSession();
  const [tab, setTab] = useSection(t.id);
  const diff = useDiffSize(t.attempt?.state === 'active' ? t.attempt.worktreeId : null);
  const inbox = useInbox();
  const accept = acceptItem(inbox.data?.items ?? [], t.id);
  const { busy: accepting, answer } = useAnswer(accept, () => setAccepting(false));
  const [acceptingOpen, setAccepting] = useState(false);
  const [commenting, setCommenting] = useState(false);
  const [making, setMaking] = useState(false);
  const [merging, setMerging] = useState(false);
  const [editing, setEditing] = useState<Editing | null>(null);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const attempt = t.attempt;

  const current = editing ? (editing.field === 'title' ? t.title : t.description) : '';
  const canSave = !!editing && editing.draft !== current && (editing.field !== 'title' || editing.draft.trim() !== '');
  const save = async () => {
    if (!editing || !canSave) return;
    setSaving(true);
    setError(null);
    try {
      await patchTicket(qc, t.id, editing.field === 'title' ? { title: editing.draft.trim() } : { description: editing.draft });
      setEditing(null);
    } catch (e) {
      setError(actionError(e));
    } finally {
      setSaving(false);
    }
  };
  const edit = (field: Editing['field']) => {
    setError(null);
    setEditing({ field, draft: field === 'title' ? t.title : t.description });
  };
  useTopBarEdit(editing ? { canSave, saving, onSave: () => void save(), onCancel: () => { setEditing(null); setError(null); } } : null);

  const watches = useWatches({ ticket: t.id }).data;
  useTopBarActions((): MenuEntry[] => [
    watchEntry(qc, { ticket: t.id }, watches),
    ...(t.waiting ? [stopWaitingEntry(qc, t)] : []),
    ...(t.pickupFailed ? [retryAutoStartEntry(qc, t)] : []),
    { submenu: 'Move to', entries: moveToEntries(qc, t, confirm) },
    { submenu: 'Priority', entries: priorityEntries(qc, t) },
    ...(t.parts && !isClosed(t.stage) ? [{ label: 'Start next part', onSelect: () => void runAction(() => startNextPart(start, t)) }] : []),
    ...(canMakeParts(t) ? [{ label: 'Make into parts…', onSelect: () => setMaking(true) }] : []),
    ...(attempt?.state === 'active' ? [{ label: 'Merge…', onSelect: () => setMerging(true) }] : []),
    ...(attempt?.state === 'active'
      ? [{ label: 'Abandon attempt…', onSelect: () => void runAction(() => abandonAttempt(confirm, qc, t, attempt)) }]
      : []),
    { separator: true },
    { label: 'Delete…', danger: true, onSelect: () => void runAction(async () => { if (await deleteTicket(confirm, qc, t)) onDeleted(); }) },
  ]);

  // Open work: the session working on it now, else the active attempt's newest one, else the ticket's tab.
  const openWork = () => {
    const session = t.agent?.sessionId ?? t.sessions.find((s) => s.attemptId === attempt?.id)?.id;
    if (session) openSession({ id: session, projectId: t.projectId });
    else openTicket(t);
  };
  const actions = phoneTicketActions(t, accept !== null);
  const action = (a: PhoneTicketAction) => {
    switch (a) {
      case 'refine':
        return <Button key={a} variant={actions.includes('start') ? 'secondary' : 'primary'} className={big} onClick={() => void runAction(() => refineTicket(qc, t))}>Refine</Button>;
      case 'start':
        return <Button key={a} variant="primary" className={big} onClick={() => start(t)}>Start</Button>;
      case 'open':
        return <Button key={a} variant="primary" className={big} onClick={openWork}>Open work</Button>;
      case 'accept':
        return <Button key={a} variant="primary" className={big} aria-haspopup="dialog" onClick={() => setAccepting(true)}>Accept…</Button>;
    }
  };
  const primaryChoice = accept?.choices.find((c) => c.primary)?.id ?? null;

  return (
    <section aria-label={`${t.key} · ${t.title}`} className="flex min-h-0 flex-1 flex-col">
      <header className="flex shrink-0 flex-col gap-1.5 px-4 pt-3 pb-2">
        {t.parent && <ParentChip parent={t.parent} projectId={t.projectId} className="self-start" />}
        {editing?.field === 'title' ? (
          <Textarea
            aria-label="Title"
            autoFocus
            rows={2}
            maxLength={200}
            value={editing.draft}
            onChange={(e) => setEditing({ field: 'title', draft: e.target.value.replace(/\n/g, ' ') })}
            onKeyDown={(e) => { if (e.key === 'Enter' && !e.nativeEvent.isComposing) { e.preventDefault(); void save(); } }}
            className="text-base font-semibold"
          />
        ) : (
          <button type="button" aria-label={`Edit title: ${t.title}`} onClick={() => edit('title')}
            className="text-left text-base font-semibold outline-none focus-visible:ring-2 focus-visible:ring-ring">
            {t.title}
          </button>
        )}
        {editing?.field === 'title' && error && <Alert tone="error">{error}</Alert>}
        <StateLine ticket={t} />
        <div className="-mx-4 overflow-x-auto px-4">
          <StageStepper stage={t.stage} className="w-max" />
        </div>
        {t.parts && <PartsStepper parts={t.parts} showCount className="w-40" />}
      </header>
      <MergedBanner ticket={t} className="border-y" />
      <HubTabs value={tab} onChange={setTab} diff={diff} />
      {tab !== 'overview' ? (
        <HubPanel ticket={t} tab={tab} />
      ) : (
        <div role="tabpanel" aria-labelledby="ticket-tab-overview" className="min-h-0 flex-1 overflow-y-auto">
          <div className="flex flex-col divide-y divide-border border-b border-border">
            <Fold title="Details" summary={detailsSummary(t)}><TicketDetails ticket={t} /></Fold>
            {(attempt || t.sessions.length > 0) && (
              <Fold title={attempt ? 'Attempt' : 'Sessions'} summary={attemptSummary(t)}><AttemptSection ticket={t} compact /></Fold>
            )}
            <Fold title="Links" summary={t.links.length === 0 ? 'None' : `${t.links.length}`}><TicketLinks ticket={t} /></Fold>
            <Fold title="Documents" summary={documents.length === 0 ? 'None' : `${documents.length}`}><TicketDocuments ticket={t} /></Fold>
            <Fold title="Attachments" summary={attachments.length === 0 ? 'None' : `${attachments.length}`}><TicketAttachments ticketId={t.id} /></Fold>
            {t.overlaps.length > 0 && <Fold title="Overlaps" summary={overlapSummary(t)}><TicketOverlaps ticket={t} /></Fold>}
            {watches && watches.length > 0 && (
              <Fold title="Watching" summary={`${watches.length}`}><Watchers of={{ ticket: t.id }} /></Fold>
            )}
          </div>
          <div className="flex flex-col divide-y divide-border px-4 *:py-4">
            <OverviewSection
              title="Description"
              actions={editing?.field !== 'description' && (
                <IconButton label="Edit description" noTooltip className="size-11" onClick={() => edit('description')}><Pencil aria-hidden /></IconButton>
              )}
            >
              {editing?.field === 'description' ? (
                <>
                  <Textarea
                    aria-label="Description"
                    autoFocus
                    rows={10}
                    value={editing.draft}
                    placeholder="Markdown: what and why, acceptance criteria…"
                    onChange={(e) => setEditing({ field: 'description', draft: e.target.value })}
                    className="min-h-48"
                  />
                  {error && <Alert tone="error">{error}</Alert>}
                </>
              ) : t.description.trim() ? (
                <Markdown breaks>{t.description}</Markdown>
              ) : (
                <p className="text-xs text-muted-foreground">No description.</p>
              )}
            </OverviewSection>
            <PartsSection ticket={t} />
            <TicketTimeline ticketId={t.id} />
          </div>
        </div>
      )}
      <div role="group" aria-label="Ticket actions" className="flex shrink-0 gap-2 border-t border-border bg-card px-2 pt-2 pb-[max(0.5rem,env(safe-area-inset-bottom))]">
        <Button variant="secondary" className={big} aria-haspopup="dialog" onClick={() => setCommenting(true)}>Comment…</Button>
        {actions.map(action)}
      </div>
      <Sheet side="bottom" title="Comment" open={commenting} onOpenChange={setCommenting}>
        <CommentComposer ticketId={t.id} rows={4} autoFocus className="" onSent={() => setCommenting(false)} />
      </Sheet>
      {acceptingOpen && accept && primaryChoice && (
        <MergeConfirm item={accept} choice={primaryChoice} busy={accepting} onCancel={() => setAccepting(false)}
          onConfirm={() => void answer({ choice: primaryChoice })} />
      )}
      {making && <MakePartsDialog ticket={t} onClose={() => setMaking(false)} />}
      {merging && attempt && <MergeDialog ticket={t} attemptId={attempt.id} onClose={() => setMerging(false)} />}
    </section>
  );
}

/** `● In progress · Medium · Coder · working`, then the badges (refining, blocked, overlap, waiting). */
function StateLine({ ticket: t }: { ticket: Ticket }) {
  const agent = t.agent ? agentLine(t.agent) : null;
  return (
    <div className="flex min-w-0 flex-wrap items-center gap-x-1.5 gap-y-1 text-xs text-muted-foreground">
      {agent && <AgentDot state={agent.dot} />}
      <span className="font-medium text-foreground">{STAGE_LABEL[t.stage]}</span>
      <span aria-hidden>·</span>
      <PriorityIcon priority={t.priority} showLabel />
      {agent && <><span aria-hidden>·</span><span>{agent.text}</span></>}
      <TicketBadges ticket={t} className="flex-wrap" />
    </div>
  );
}

/** A folding block with its one-line summary; closed at first. */
function Fold({ title, summary, children }: { title: string; summary: string; children: ReactNode }) {
  const [open, setOpen] = useState(false);
  return (
    <div>
      <button
        type="button"
        aria-expanded={open}
        onClick={() => setOpen((o) => !o)}
        className="flex min-h-12 w-full min-w-0 items-center gap-2 px-4 text-left text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset"
      >
        {open ? <ChevronDown aria-hidden className="size-4 shrink-0 text-muted-foreground" /> : <ChevronRight aria-hidden className="size-4 shrink-0 text-muted-foreground" />}
        <span className="shrink-0 font-medium">{title}</span>
        <span className="min-w-0 flex-1 truncate text-xs text-muted-foreground">{summary}</span>
      </button>
      {open && (
        <div className="px-4 pb-3">
          <FoldedSectionContext.Provider value>{children}</FoldedSectionContext.Provider>
        </div>
      )}
    </div>
  );
}

function detailsSummary(t: Ticket): string {
  return [STAGE_LABEL[t.stage], PRIORITY_LABEL[t.priority], ...t.labels, ...t.areas].join(' · ');
}

function attemptSummary(t: Ticket): string {
  const a = t.attempt;
  if (!a) return `${t.sessions.length} session${t.sessions.length === 1 ? '' : 's'}`;
  const ci = a.pr?.ci ?? a.ci;
  return [
    a.branch,
    a.state === 'active' ? null : a.state,
    a.pr ? `${prName(a.pr)}${a.pr.state === 'open' ? '' : ` ${a.pr.state}`}` : null,
    ci ? `CI ${ci.state}` : null,
  ].filter(Boolean).join(' · ');
}

function overlapSummary(t: Ticket): string {
  const conflicts = t.overlaps.filter((o) => o.level === 'conflict').length;
  return [conflicts > 0 ? `${conflicts} conflict${conflicts === 1 ? '' : 's'}` : null, t.overlaps.map((o) => o.key).join(', ')]
    .filter(Boolean).join(' · ');
}
