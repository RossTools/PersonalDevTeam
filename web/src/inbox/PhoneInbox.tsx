// The Inbox on a phone (PDT-117, design §6): the list with Needs you | Updates and Review all, and one item per screen
// with its decision fixed at the bottom, in the thumb's reach.
import { useEffect, useRef, useState, type ReactNode, type TouchEvent } from 'react';
import { useQuery } from '@tanstack/react-query';
import { ChevronDown } from 'lucide-react';
import { ApprovalCard, Button, Dialog, DiffViewer, EmptyState, Sheet, Skeleton, Textarea, relativeTime } from '@ui';
import { cn } from '@ui/lib/utils';
import { api } from '../api/client';
import type { InboxItem } from '../api/gen/InboxItem';
import type { Notification } from '../api/gen/Notification';
import type { NotificationTarget } from '../api/gen/NotificationTarget';
import { Markdown } from '../common/Markdown';
import { approvalTitle, questionsOf as requestQuestions } from '../conversation/tools';
import { useTopBarActions } from '../shell/PhoneTopBar';
import { useOpenSession, useOpenTicket } from '../tickets/actions';
import { PrDetails } from '../tickets/PullRequest';
import { useTicket } from '../tickets/queries';
import { useOpenChanges } from '../review/open';
import { answerApproval, UpdateView } from './InboxView';
import { after, dismissable, KIND_LABEL, merges, passesOf, questionsOf, selectedUpdate, step } from './model';
import { useAnswer } from './useAnswer';

/** The bottom bar's buttons: 48px, whatever the pointer. */
const big = 'h-12 pointer-coarse:h-12 min-w-0 flex-1';
const capital = (s: string) => s.charAt(0).toUpperCase() + s.slice(1);

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
    <Button key={d.id} variant={d.primary ? 'primary' : 'secondary'} className={big} pending={busy && d.primary}
      disabled={busy || d.disabled} onClick={() => onPick(d.id)}>
      <span className="truncate">{d.title}</span>
    </Button>
  );
  return (
    <div role="group" aria-label="Decision" className="flex gap-2 border-t border-border bg-card px-2 pt-2 pb-[max(0.5rem,env(safe-area-inset-bottom))]">
      {folded.length > 0 && (
        <>
          <Button variant="secondary" className={big} aria-haspopup="dialog" disabled={busy} onClick={() => setMore(true)}>
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
          className={cn('h-12 w-full justify-start pointer-coarse:h-12', selected === r.id && 'border-primary bg-accent')}
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
  useTopBarActions(item ? () => [
    ...(item.sessionId && item.projectId ? [{ label: 'Open session', onSelect: () => openSession({ id: item.sessionId!, projectId: item.projectId! }) }] : []),
    ...(item.ticketId && item.projectId ? [{ label: 'Open ticket', onSelect: () => openTicket({ id: item.ticketId!, projectId: item.projectId! }) }] : []),
    ...(dismissable(item) ? [{ label: 'Dismiss', onSelect: () => void dismiss() }] : []),
  ] : null);

  // Opening an item reads its notifications.
  const readId = item && !item.id.startsWith('session:') ? item.id : null;
  useEffect(() => {
    if (readId) void api.readNotifications({ inboxItemId: readId }).catch(() => {});
  }, [readId]);
  // An item answered elsewhere: show the first one left.
  const gone = !updateId && !item && !loading && live.length > 0 && !done.has(itemId);
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
    return update ? <UpdateView update={update} onOpenTarget={onOpenTarget} /> : <EmptyState>This update is gone.</EmptyState>;
  }
  if (!item) {
    if (loading) return <Skeleton lines={8} className="p-4" />;
    if (live.length > 0) return <Skeleton lines={8} className="p-4" />;
    return (
      <EmptyState action={<Button variant="primary" className="h-12 px-6 pointer-coarse:h-12" onClick={onList}>Back to Inbox</Button>}>
        <span className="text-base font-medium text-foreground">All clear</span>
        <br />Nothing else waits on you.
      </EmptyState>
    );
  }
  return (
    <div className="flex min-h-0 flex-1 flex-col overflow-hidden" onTouchStart={onTouchStart} onTouchEnd={onTouchEnd}>
      <TriageItem
        key={item.id}
        item={item}
        busy={busy}
        className={cn('animate-in duration-200 motion-reduce:animate-none', dir === 1 ? 'slide-in-from-right' : 'slide-in-from-left')}
        onAnswer={answer}
        onApproved={() => finish(item)}
        onOpenSession={() => item.sessionId && item.projectId && openSession({ id: item.sessionId, projectId: item.projectId })}
      />
    </div>
  );
}

/** One item: its context and content (scrolling), and its decisions at the bottom. Keyed by item, so its state is its own. */
function TriageItem({ item, busy, className, onAnswer, onApproved, onOpenSession }: {
  item: InboxItem;
  busy: boolean;
  className?: string;
  onAnswer: (a: { choice: string; text?: string } | { answers: string[] }) => Promise<void>;
  onApproved: () => void;
  onOpenSession: () => void;
}) {
  const [note, setNote] = useState('');
  const [answers, setAnswers] = useState<string[]>([]);
  const [takingOver, setTakingOver] = useState(false);
  const [confirming, setConfirming] = useState<string | null>(null);
  const [approving, setApproving] = useState(false);
  const withNote = (choice: string) => void onAnswer({ choice, ...(note.trim() ? { text: note } : {}) });
  const pick = (choice: string) => (merges(item, choice) ? setConfirming(choice) : withNote(choice));
  const itemChoices: Decision[] = item.choices.map((c) => ({ id: c.id, title: capital(c.title), primary: c.primary }));

  let content: ReactNode = null;
  let bar: ReactNode = null;
  const noteBox = (label: string, placeholder: string) => (
    <Textarea aria-label={label} placeholder={placeholder} value={note} onChange={(e) => setNote(e.target.value)} rows={3} />
  );
  switch (item.kind) {
    case 'question': {
      const qs = questionsOf(item);
      const set = (n: number, v: string) => setAnswers(qs.map((_, i) => (i === n ? v : answers[i] ?? '')));
      content = qs.map((q, n) => (
        <div key={n} className="flex flex-col gap-2">
          <div className="text-sm font-medium"><Markdown>{q.question}</Markdown></div>
          {q.choices.length > 0 && (
            <Rows label={`Choices for question ${n + 1}`} rows={q.choices.map((c) => ({ id: c, title: c }))} selected={answers[n] ?? ''}
              onPick={(c) => set(n, c)} />
          )}
          <Textarea aria-label={`Answer ${n + 1}`} placeholder={q.choices.length > 0 ? 'Or your own answer' : 'Your answer'}
            value={answers[n] ?? ''} onChange={(e) => set(n, e.target.value)} rows={2} />
        </div>
      ));
      bar = <DecisionBar busy={busy} decisions={[{ id: 'send', title: 'Send', primary: true, disabled: !answers.some((a) => a?.trim()) }]}
        onPick={() => void onAnswer({ answers })} />;
      break;
    }
    case 'approval': {
      const tool = typeof item.body.tool === 'string' ? item.body.tool : 'tool';
      const diff = typeof item.body.diff === 'string' ? item.body.diff : null;
      const description = typeof item.body.description === 'string' ? item.body.description : null;
      const questions = requestQuestions(item.body.input);
      if (questions.length > 0) {
        // The agent's own questions (AskUserQuestion): the card asks them and sends the answers.
        content = <ApprovalCard kind="question" title={approvalTitle(tool)} questions={questions}
          onAnswer={(a) => answerApproval(item, a, onApproved)} />;
        break;
      }
      content = (
        <>
          <p className="text-sm font-medium">{approvalTitle(tool)}</p>
          {diff ? <DiffViewer diff={diff} /> : <pre className="overflow-auto rounded-md bg-muted p-2 font-mono text-xs whitespace-pre-wrap">{item.title}</pre>}
          {description && <p className="text-sm text-muted-foreground">{description}</p>}
        </>
      );
      const send = (id: string) => {
        setApproving(true);
        const a = id === 'deny' ? { type: 'deny' as const } : { type: 'approve' as const, always: id === 'always' };
        void answerApproval(item, a, onApproved).finally(() => setApproving(false));
      };
      bar = <DecisionBar busy={approving} onPick={send} decisions={[
        { id: 'deny', title: 'Deny' },
        ...(item.choices.some((c) => c.id === 'allow_always') ? [{ id: 'always', title: 'Always' }] : []),
        { id: 'allow', title: 'Allow', primary: true },
      ]} />;
      break;
    }
    case 'session':
      content = (
        <p className="text-sm">
          {typeof item.body.sessionTitle === 'string' ? item.body.sessionTitle : 'This session'} waits on a permission prompt
          or a question. Answer it in the session; looking at it takes it out of the Inbox.
        </p>
      );
      bar = <DecisionBar busy={false} decisions={[{ id: 'open', title: 'Open session', primary: true, disabled: !item.sessionId }]} onPick={onOpenSession} />;
      break;
    case 'stalled': {
      const outcomes = item.choices.filter((c) => c.id.startsWith('outcome:'));
      content = (
        <>
          <p className="text-sm">
            {typeof item.body.why === 'string' ? item.body.why : 'The step stopped without an outcome.'} Nudge it to carry
            on, or take it over and pick the outcome yourself.
          </p>
          {noteBox('Nudge message (optional)', 'Your own words for the nudge (optional)')}
          <Sheet side="bottom" title="Take over: the step ends with" open={takingOver} onOpenChange={setTakingOver}>
            <Rows rows={outcomes.map((c) => ({ id: c.id, title: c.id.slice('outcome:'.length) }))} disabled={busy}
              onPick={(id) => { setTakingOver(false); void onAnswer({ choice: id }); }} />
          </Sheet>
        </>
      );
      bar = <DecisionBar busy={busy} decisions={[
        { id: 'takeover', title: 'Take over', disabled: outcomes.length === 0 },
        { id: 'nudge', title: 'Nudge', primary: true },
      ]} onPick={(id) => (id === 'takeover' ? setTakingOver(true) : withNote('nudge'))} />;
      break;
    }
    case 'loop_limit':
      content = (
        <>
          <p className="text-sm">This step has run as many times as its loop limit allows. Its passes so far:</p>
          <ol className="flex flex-col gap-2">
            {passesOf(item).map((p, n) => (
              <li key={n} className="rounded-md border border-border p-2 text-sm">
                <span className="font-medium">{p.persona}</span>: {p.outcome ?? 'no outcome'}
                {p.summary && <p className="mt-1 text-muted-foreground">{p.summary}</p>}
              </li>
            ))}
          </ol>
        </>
      );
      bar = <DecisionBar busy={busy} decisions={itemChoices} onPick={(choice) => void onAnswer({ choice })} />;
      break;
    case 'timed_out':
      content = (
        <>
          <p className="text-sm">
            {typeof item.body.node === 'string' ? `Step ${item.body.node}` : 'The step'} ended without an outcome:{' '}
            {typeof item.body.why === 'string' ? item.body.why : 'it ran out of time'}. Resume runs it again on the same
            ticket and attempt; Stop leaves it stopped.
          </p>
          {noteBox('Note (optional)', 'A note for the agent with Resume (optional)')}
        </>
      );
      bar = <DecisionBar busy={busy} decisions={itemChoices}
        onPick={(choice) => void onAnswer({ choice, ...(choice === 'resume' && note.trim() ? { text: note } : {}) })} />;
      break;
    case 'manual':
      content = (
        <>
          <p className="text-sm">This stage is yours: do it, then pick how it ended.</p>
          {typeof item.body.prompt === 'string' && (
            <div className="rounded-md border border-border p-3 text-sm"><Markdown>{item.body.prompt}</Markdown></div>
          )}
          {noteBox('What you did (optional)', 'What you did or found (optional): the next step reads it')}
        </>
      );
      bar = <DecisionBar busy={busy} decisions={itemChoices} onPick={withNote} />;
      break;
    case 'gate':
      content = (
        <>
          {noteBox('Note (optional)', item.ticketId ? 'A note with your choice (optional), e.g. what to change when you reject it' : 'A note with your choice (optional)')}
          {item.ticketId && <p className="text-meta text-muted-foreground">A note goes on the ticket as your comment, so the agent working on it gets it.</p>}
        </>
      );
      bar = <DecisionBar busy={busy} decisions={itemChoices} onPick={pick} />;
      break;
  }

  const step = [typeof item.body.workflow === 'string' ? item.body.workflow : null, typeof item.body.node === 'string' && item.kind !== 'timed_out' ? item.body.node : null]
    .filter(Boolean).join(' · ');
  const meta = [KIND_LABEL[item.kind], item.personaName, relativeTime(item.createdAt)].filter(Boolean).join(' · ');
  return (
    <>
      <div className={cn('flex min-h-0 flex-1 flex-col gap-4 overflow-auto p-4', className)}>
        <header className="flex flex-col gap-1">
          {item.ticketKey && <p className="truncate text-xs text-muted-foreground">{item.ticketKey}{item.ticketTitle ? ` · ${item.ticketTitle}` : ''}</p>}
          <h2 className="text-base font-medium">{item.title}</h2>
          <p className="text-meta text-muted-foreground">{meta}{step ? ` · ${step}` : ''}</p>
        </header>
        {content}
      </div>
      {bar}
      {confirming && (
        <MergeConfirm item={item} choice={confirming} busy={busy} onCancel={() => setConfirming(null)}
          onConfirm={() => { setConfirming(null); withNote(confirming); }} />
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
  const changes = useQuery({ queryKey: ['changes', worktreeId], queryFn: () => api.getChanges(worktreeId!), enabled: !!worktreeId });
  const files = changes.data?.files ?? [];
  const added = files.reduce((n, f) => n + f.added, 0);
  const removed = files.reduce((n, f) => n + f.removed, 0);
  const title = capital(item.choices.find((c) => c.id === choice)?.title ?? choice);
  const ci = attempt?.pr?.ci ?? attempt?.ci ?? null;
  return (
    <Dialog
      open
      onOpenChange={(o) => !o && onCancel()}
      title={`${title} ${item.ticketKey ?? 'this'}?`}
      description={item.ticketTitle ?? item.title}
      footer={
        <>
          <Button className="h-12 pointer-coarse:h-12" onClick={onCancel}>Cancel</Button>
          <Button variant="primary" className="h-12 pointer-coarse:h-12" pending={busy} onClick={onConfirm}>{title}</Button>
        </>
      }
    >
      <div className="flex flex-col gap-3 text-sm" aria-label="What merges">
        {ticket.isLoading ? <Skeleton lines={2} /> : attempt?.pr ? (
          <PrDetails pr={attempt.pr} />
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
              {changes.isLoading ? 'Counting the changes…' : `${files.length} ${files.length === 1 ? 'file' : 'files'}, +${added} −${removed}`}
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
