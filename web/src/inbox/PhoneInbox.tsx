// The Inbox on a phone (PDT-117): the list with Needs you | Updates and Review all, and one item per screen
// with its decision fixed at the bottom, in the thumb's reach.
import { useEffect, useRef, useState, type TouchEvent } from 'react';
import { useQuery } from '@tanstack/react-query';
import { ChevronDown } from 'lucide-react';
import { Button, Dialog, EmptyState, Sheet, Skeleton, Textarea } from '@ui';
import { cn } from '@ui/lib/utils';
import { api } from '../api/client';
import type { InboxAnswer } from '../api/gen/InboxAnswer';
import type { InboxItem } from '../api/gen/InboxItem';
import type { Notification } from '../api/gen/Notification';
import type { NotificationTarget } from '../api/gen/NotificationTarget';
import { ErrorBoundary } from '../common/ErrorBoundary';
import { useTopBarActions } from '../shell/PhoneTopBar';
import { useOpenSession, useOpenTicket, useOpenChanges, openExternal } from '../common/open';
import { PrDetails } from '../tickets/PullRequest';
import { useProjectForge, useTicket } from '../tickets/queries';
import { UpdateView } from './InboxView';
import { after, changeRequestLabel, dismissable, inboxTargets, merges, selectedUpdate, step } from './model';
import { useAnswer } from './useAnswer';
import { useItemView } from './itemView';
import { ItemHeader, Why } from './ItemLayout';
import { capitalize, plural } from '../common/text';
import { changesKey } from '../review/queries';

/** Opens the pull request a gate is about on its forge, in a new tab. */
const openChangeRequest = (item: InboxItem) => item.changeRequest && openExternal(item.changeRequest.url);

/** A decision in the bottom bar. */
interface Decision {
  id: string;
  title: string;
  primary?: boolean;
  disabled?: boolean;
}

/**
 * The bottom bar: the decisions, the primary one on the right. More than three: the primary one and **More ▾**, a
 * bottom sheet with the rest as big rows.
 */
export function DecisionBar({ decisions, busy, onPick }: { decisions: Decision[]; busy: boolean; onPick: (id: string) => void }) {
  const [more, setMore] = useState(false);
  const ordered = [...decisions.filter((d) => !d.primary), ...decisions.filter((d) => d.primary)];
  const main = ordered.at(-1);
  const folded = ordered.length > 3 && main ? ordered.slice(0, -1) : [];
  const shown = folded.length > 0 && main ? [main] : ordered;
  const button = (d: Decision) => (
    <Button key={d.id} variant={d.primary ? 'primary' : 'secondary'} size="lg" className="min-w-0 flex-1" pending={busy && d.primary}
      disabled={busy || d.disabled} onClick={() => onPick(d.id)}>
      <span className="truncate">{d.title}</span>
    </Button>
  );
  return (
    <div role="group" aria-label="Decision" className="flex gap-2 border-t border-border bg-card px-2 pt-2 pb-[max(0.5rem,env(safe-area-inset-bottom))]">
      {folded.length > 0 && (
        <>
          <Button variant="secondary" size="lg" className="min-w-0 flex-1" aria-haspopup="dialog" disabled={busy} onClick={() => setMore(true)}>
            More <ChevronDown aria-hidden />
          </Button>
          <Sheet side="bottom" title="More choices" open={more} onOpenChange={setMore}>
            <Rows rows={folded} disabled={busy} onPick={(id) => { setMore(false); onPick(id); }} />
          </Sheet>
        </>
      )}
      {shown.map(button)}
    </div>
  );
}

/** Choices as full-width 48px rows (a question's answers, More ▾, Take over's outcomes). */
function Rows({ rows, selected, disabled, onPick, label }: {
  rows: Decision[];
  selected?: string;
  disabled?: boolean;
  onPick: (id: string) => void;
  label?: string;
}) {
  return (
    <div role="group" aria-label={label} className="flex flex-col gap-1.5">
      {rows.map((r) => (
        <Button key={r.id} variant="secondary" disabled={disabled || r.disabled}
          {...(selected !== undefined ? { 'aria-pressed': selected === r.id } : {})}
          size="lg" className={cn('w-full justify-start px-3', selected === r.id && 'border-primary bg-accent')}
          onClick={() => onPick(r.id)}>
          <span className="truncate">{r.title}</span>
        </Button>
      ))}
    </div>
  );
}

/**
 * `/inbox/:itemId` on a phone: one item, its ticket and step in brief, the kind's content (it scrolls) and its
 * decisions fixed at the bottom. After a decision the next item slides in; "All clear" at the end. A swipe moves
 * between items without answering. ⋯ has Open session, Open ticket and Dismiss.
 */
export function PhoneInboxItem({ items, updates, itemId, loading, onMove, onList, onOpenTarget }: {
  items: InboxItem[];
  updates: Notification[];
  itemId: string;
  loading?: boolean;
  /** Shows another item in place of this one (Back still returns to the list). */
  onMove: (id: string) => void;
  onList: () => void;
  onOpenTarget?: (t: NotificationTarget) => void;
}) {
  // Items answered here stay away until the Inbox is fetched again.
  const [done, setDone] = useState<ReadonlySet<string>>(new Set());
  const live = items.filter((i) => !done.has(i.id));
  const updateId = selectedUpdate(itemId);
  const update = updateId ? updates.find((u) => u.id === updateId) ?? null : null;
  const item = updateId ? null : live.find((i) => i.id === itemId) ?? null;
  // Which way the item came in: 1 from the right (next), -1 from the left (back).
  const [dir, setDir] = useState<1 | -1>(1);
  const move = (id: string | null, by: 1 | -1) => {
    if (!id || id === itemId) return;
    setDir(by);
    onMove(id);
  };
  const { busy, answer, dismiss } = useAnswer(item, (d) => finish(d));
  const finish = (d: InboxItem) => {
    setDone((s) => new Set(s).add(d.id));
    move(after(live, d.id), 1);
  };
  const openTicket = useOpenTicket();
  const openSession = useOpenSession();
  useTopBarActions(item ? () => {
    const { ticket, session } = inboxTargets(item);
    return [
    ...(session ? [{ label: 'Open session', onSelect: () => openSession(session) }] : []),
    ...(ticket ? [{ label: 'Open ticket', onSelect: () => openTicket(ticket) }] : []),
    ...(item.changeRequest ? [{ label: changeRequestLabel(item)!, onSelect: () => openChangeRequest(item) }] : []),
    ...(dismissable(item) ? [{ label: 'Dismiss', onSelect: () => void dismiss() }] : []),
    ];
  } : null);

  // Opening an item reads its notifications.
  const readId = item && !item.id.startsWith('session:') ? item.id : null;
  useEffect(() => {
    if (readId) void api.readNotifications({ inboxItemId: readId }).catch(() => {});
  }, [readId]);
  // An item answered elsewhere, or one answered here when another has come in since: show the first one left.
  const gone = !updateId && !item && !loading && live.length > 0;
  const firstLeft = gone ? step(live, null, 1) : null;
  useEffect(() => {
    if (firstLeft) onMove(firstLeft);
  }, [firstLeft, onMove]);

  const touch = useRef<{ x: number; y: number } | null>(null);
  const onTouchStart = (e: TouchEvent) => {
    const t = e.target as Element;
    // Text and wide content (a diff, code) keep their own gestures.
    touch.current = t.closest('textarea, input, pre, [data-slot=diff]') ? null : { x: e.touches[0].clientX, y: e.touches[0].clientY };
  };
  const onTouchEnd = (e: TouchEvent) => {
    const from = touch.current;
    touch.current = null;
    if (!from || !item) return;
    const dx = e.changedTouches[0].clientX - from.x;
    const dy = e.changedTouches[0].clientY - from.y;
    if (Math.abs(dx) < 60 || Math.abs(dx) < 2 * Math.abs(dy)) return;
    const by = dx < 0 ? 1 : -1;
    move(step(live, item.id, by), by);
  };

  if (updateId) {
    return update ? <UpdateView update={update} onOpenTarget={onOpenTarget} onCleared={onList} /> : <EmptyState>This update is gone.</EmptyState>;
  }
  if (!item) {
    if (loading) return <Skeleton lines={8} className="p-4" />;
    if (live.length > 0) return <Skeleton lines={8} className="p-4" />;
    return (
      <EmptyState action={<Button variant="primary" size="lg" className="px-6" onClick={onList}>Back to Inbox</Button>}>
        <span className="text-base font-medium text-foreground">All clear</span>
        <br />Nothing else waits on you.
      </EmptyState>
    );
  }
  return (
    <div className="flex min-h-0 flex-1 flex-col overflow-hidden" onTouchStart={onTouchStart} onTouchEnd={onTouchEnd}>
      <ErrorBoundary key={item.id} onBack={onList}>
        <TriageItem
          item={item}
          busy={busy}
          className={cn('animate-in duration-200 motion-reduce:animate-none', dir === 1 ? 'slide-in-from-right' : 'slide-in-from-left')}
          onAnswer={answer}
          onApproved={() => finish(item)}
          onDismiss={() => void dismiss()}
        />
      </ErrorBoundary>
    </div>
  );
}

/**
 * One item in the one item layout (PDT-361): its header and links, the line on why, its content and note box
 * (scrolling), and its actions in the bottom bar. Keyed by item, so its state is its own.
 */
function TriageItem({ item, busy, className, onAnswer, onApproved, onDismiss }: {
  item: InboxItem;
  busy: boolean;
  className?: string;
  onAnswer: (a: InboxAnswer) => Promise<void>;
  onApproved: () => void;
  onDismiss: () => void;
}) {
  const [confirming, setConfirming] = useState<string | null>(null);
  const { spec, note, setNote } = useItemView({ item, busy, answer: onAnswer, dismiss: onDismiss, onApproved, phone: true });
  // The bar's order: the rest first, the primary one on the right; Dismiss and the like before them. Dismiss is in ⋯,
  // except for a notice, whose bar has it as its one other way out.
  const actions = [...spec.actions.filter((a) => a.end), ...spec.actions.filter((a) => !a.end)]
    .filter((a) => a.id !== 'dismiss' || item.kind === 'notice');
  const decisions: Decision[] = actions.map((a) => ({ id: a.id, title: a.short ?? a.title, primary: a.primary, disabled: a.disabled }));
  const pick = (id: string) => {
    // Merge (and Accept) asks first.
    if (merges(item, id)) return setConfirming(id);
    if (id === 'takeover') return spec.takeOver?.setOpen(true);
    actions.find((a) => a.id === id)?.run();
  };
  const pending = spec.actions.some((a) => a.pending);
  return (
    <>
      <div className={cn('flex min-h-0 flex-1 flex-col gap-4 overflow-auto p-4', className)}>
        <ItemHeader item={item} spec={spec} className="pb-3" />
        <section aria-label={spec.region} className="flex flex-col gap-4">
          {spec.why && <Why>{spec.why}</Why>}
          {spec.content}
        </section>
        {spec.note && (
          <div className="flex flex-col gap-1">
            <Textarea aria-label={spec.note.label} placeholder={spec.note.placeholder} value={note} onChange={(e) => setNote(e.target.value)} rows={3} />
            {spec.note.hint && <p className="text-meta text-muted-foreground">{spec.note.hint}</p>}
          </div>
        )}
        {spec.hint && <p className="text-meta text-muted-foreground">{spec.hint}</p>}
        {spec.takeOver && (
          <Sheet side="bottom" title="Take over: the step ends with" open={spec.takeOver.open} onOpenChange={spec.takeOver.setOpen}>
            <Rows rows={spec.takeOver.outcomes} disabled={busy} onPick={(id) => spec.takeOver!.pick(id)} />
          </Sheet>
        )}
      </div>
      {decisions.length > 0 && <DecisionBar busy={pending} decisions={decisions} onPick={pick} />}
      {confirming && (
        <MergeConfirm item={item} choice={confirming} busy={busy} onCancel={() => setConfirming(null)}
          onConfirm={() => void onAnswer({ choice: confirming, ...(note.trim() ? { text: note } : {}) }).finally(() => setConfirming(null))} />
      )}
    </>
  );
}

/** Merge (and Accept) asks first: the PR, its CI and the size of the change, with a way to look at it. */
export function MergeConfirm({ item, choice, busy, onCancel, onConfirm }: {
  item: InboxItem;
  choice: string;
  busy: boolean;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  const ticket = useTicket(item.ticketId);
  const openChanges = useOpenChanges();
  const t = ticket.data;
  const attempt = t?.attempts.find((x) => x.id === item.attemptId) ?? t?.attempts.find((x) => x.state === 'active');
  const worktreeId = attempt?.worktreeId ?? null;
  const changes = useQuery({ queryKey: changesKey(worktreeId ?? ''), queryFn: () => api.getChanges(worktreeId!), enabled: !!worktreeId });
  const files = changes.data?.files ?? [];
  const added = files.reduce((n, f) => n + f.added, 0);
  const removed = files.reduce((n, f) => n + f.removed, 0);
  const title = capitalize(item.choices.find((c) => c.id === choice)?.title ?? choice);
  const ci = attempt?.pr?.ci ?? attempt?.ci ?? null;
  const queueName = useProjectForge(attempt?.pr ? t?.projectId ?? null : null).data?.queueName ?? undefined;
  return (
    <Dialog
      onClose={onCancel}
      title={`${title} ${item.ticketKey ?? 'this'}?`}
      description={item.ticketTitle ?? item.title}
      footer={
        <>
          <Button size="lg" onClick={onCancel}>Cancel</Button>
          <Button variant="primary" size="lg" pending={busy} onClick={onConfirm}>{title}</Button>
        </>
      }
    >
      <div className="flex flex-col gap-3 text-sm" aria-label="What merges">
        {ticket.isLoading ? <Skeleton lines={2} /> : attempt?.pr ? (
          <PrDetails pr={attempt.pr} {...(queueName ? { queueName } : {})} />
        ) : (
          <p className="text-muted-foreground">
            No pull request{attempt ? `: ${attempt.branch} merges into ${attempt.baseBranch}` : ''}.
            {!attempt?.pr && ci ? ` CI: ${ci.state}.` : ''}
          </p>
        )}
        {attempt?.pr && !attempt.pr.ci && <p className="text-muted-foreground">No CI on this pull request yet.</p>}
        {worktreeId && (
          <div className="flex flex-wrap items-center gap-2">
            <span aria-label="Diff size">
              {changes.isLoading ? 'Counting the changes…' : `${plural(files.length, 'file')}, +${added} −${removed}`}
            </span>
            <Button variant="ghost" size="sm" onClick={() => { onCancel(); openChanges({ id: worktreeId, projectId: item.projectId ?? '' }, { ticketId: item.ticketId ?? undefined }); }}>
              View changes
            </Button>
          </div>
        )}
      </div>
    </Dialog>
  );
}
