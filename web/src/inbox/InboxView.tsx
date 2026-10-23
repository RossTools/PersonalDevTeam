import { useEffect, useRef, useState } from 'react';
import { ApprovalCard, Button, DiffViewer, EmptyState, Kbd, Textarea, ViewHeader, relativeTime, toast, type ApprovalAnswer } from '@ui';
import { api, ApiError } from '../api/client';
import type { InboxItem } from '../api/gen/InboxItem';
import type { Notification } from '../api/gen/Notification';
import type { NotificationTarget } from '../api/gen/NotificationTarget';
import { Markdown } from '../common/Markdown';
import { TicketKey } from '../common/TicketKey';
import { actionError } from '../common/useAction';
import { approvalTitle, questionsOf as requestQuestions } from '../conversation/tools';
import { useOpenSession, useOpenTicket } from '../tickets/actions';
import { useOpenChanges } from '../review/open';
import { parseChoice, RunMenu, useRunChoices } from '../runs/RunMenu';
import { useStartRun } from '../runs/useStartRun';
import { useTicket } from '../tickets/queries';
import { useTicketTabs } from '../workspaces/ticketTabs';
import { after, dismissable, KIND_LABEL, passesOf, primary, questionsOf, selectedUpdate, step } from './model';
import { useReadNotification } from '../notifications/queries';
import { useAnswer } from './useAnswer';

/** Keys typed into a field, or with a dialog or menu open, are not the Inbox's. */
function foreign(e: KeyboardEvent): boolean {
  const t = e.target as HTMLElement | null;
  if (e.ctrlKey || e.metaKey || e.altKey) return true;
  if (t && (t.isContentEditable || ['INPUT', 'TEXTAREA', 'SELECT'].includes(t.tagName))) return true;
  return !!document.querySelector('[role=dialog], [role=menu], [role=listbox]');
}

/**
 * The Inbox's work area (UX spec §8.4): the selected item and its decision, with the ticket's key and title and a
 * way to its ticket and session. Triage keys: `j`/`k` move, Enter takes the primary action, `o` opens the ticket,
 * `e` dismisses where that makes sense; answering moves to the next item. An Update (PDT-108) shows what it says and
 * opens what it is about.
 */
export function InboxView({ items, updates = [], selectedId, onSelect, onOpenTarget }: {
  items: InboxItem[];
  updates?: Notification[];
  selectedId: string | null;
  onSelect: (id: string | null) => void;
  onOpenTarget?: (t: NotificationTarget) => void;
}) {
  const updateId = selectedUpdate(selectedId);
  const update = updateId ? updates.find((u) => u.id === updateId) ?? null : null;
  const item = updateId ? null : items.find((i) => i.id === selectedId) ?? null;
  const openTicket = useOpenTicket();
  const openSession = useOpenSession();
  const { busy, answer, dismiss, refresh } = useAnswer(item, (done) => onSelect(after(items, done.id)));
  // Questions: one answer per question, kept per item while it is on screen.
  const [answers, setAnswers] = useState<Record<string, string[]>>({});
  const [nudge, setNudge] = useState('');
  const [takingOver, setTakingOver] = useState(false);

  // An item that went away (answered elsewhere): show the first one left.
  useEffect(() => {
    if (!item && !updateId && items.length > 0) onSelect(step(items, null, 1));
  }, [item, updateId, items, onSelect]);
  // Opening an item reads its notifications.
  const itemId = item && !item.id.startsWith('session:') ? item.id : null;
  useEffect(() => {
    if (itemId) void api.readNotifications({ inboxItemId: itemId }).catch(() => {});
  }, [itemId]);
  useEffect(() => {
    setTakingOver(false);
    setNudge('');
  }, [selectedId]);

  const questionAnswers = item ? answers[item.id] ?? [] : [];
  const sendAnswers = () => void answer({ answers: questionAnswers });
  const doPrimary = () => {
    if (!item || busy) return;
    if (item.kind === 'question') {
      if (questionAnswers.some((a) => a.trim())) sendAnswers();
      return;
    }
    if (item.kind === 'stalled') return void answer({ choice: 'nudge', ...(nudge.trim() ? { text: nudge } : {}) });
    const p = primary(item);
    // A manual step: the user's note (the nudge box) is the outcome's summary.
    if (p && item.kind === 'manual') return void answer({ choice: p, ...(nudge.trim() ? { text: nudge } : {}) });
    // A gate's note (the same box) goes with its choice, and a resumed step's to its session.
    if (p && (item.kind === 'gate' || item.kind === 'timed_out')) return void answer({ choice: p, ...(nudge.trim() ? { text: nudge } : {}) });
    if (item.kind === 'session') {
      if (item.sessionId && item.projectId) openSession({ id: item.sessionId, projectId: item.projectId });
      return;
    }
    if (p && item.kind !== 'approval') void answer({ choice: p });
  };

  // The handlers change every render; the listener reads the latest through a ref.
  const keys = useRef({ doPrimary, dismiss, item, items, onSelect, openTicket });
  keys.current = { doPrimary, dismiss, item, items, onSelect, openTicket };
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (foreign(e)) return;
      const k = keys.current;
      switch (e.key) {
        case 'j': k.onSelect(step(k.items, k.item?.id ?? null, 1)); break;
        case 'k': k.onSelect(step(k.items, k.item?.id ?? null, -1)); break;
        case 'Enter': k.doPrimary(); break;
        case 'o':
          if (k.item?.ticketId && k.item.projectId) k.openTicket({ id: k.item.ticketId, projectId: k.item.projectId });
          break;
        case 'e': void k.dismiss(); break;
        default: return;
      }
      e.preventDefault();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, []);

  if (updateId) {
    return update ? <UpdateView update={update} onOpenTarget={onOpenTarget} /> : <EmptyState>This update is gone.</EmptyState>;
  }
  if (!item) {
    return <EmptyState>{items.length === 0 ? 'Nothing waits on you. Questions, approvals and gates from agents and workflows show up here.' : 'Choose an item.'}</EmptyState>;
  }

  const meta = [KIND_LABEL[item.kind], item.personaName, relativeTime(item.createdAt)].filter(Boolean).join(' · ');
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <ViewHeader
        title={item.title}
        titleTooltip={item.title}
        leading={item.ticketKey ? <TicketKey ticketKey={item.ticketKey} /> : undefined}
        meta={<span className="text-meta text-muted-foreground">{meta}</span>}
        actions={
          <>
            {item.ticketId && item.projectId && (
              <Button variant="ghost" size="sm" onClick={() => openTicket({ id: item.ticketId!, projectId: item.projectId! })}>
                Open ticket <Kbd>o</Kbd>
              </Button>
            )}
            {item.sessionId && item.projectId && item.kind !== 'session' && (
              <Button variant="ghost" size="sm" onClick={() => openSession({ id: item.sessionId!, projectId: item.projectId! })}>
                Open session
              </Button>
            )}
            {dismissable(item) && (
              <Button variant="ghost" size="sm" onClick={() => void dismiss()}>Dismiss <Kbd>e</Kbd></Button>
            )}
          </>
        }
      />
      <div className="flex min-h-0 flex-1 flex-col gap-4 overflow-auto p-4">
        {item.ticketTitle && <p className="text-sm text-muted-foreground">{item.ticketKey} · {item.ticketTitle}</p>}
        <TicketWork item={item} />
        {item.kind === 'question' && (
          <Questions
            item={item}
            answers={questionAnswers}
            onChange={(a) => setAnswers((all) => ({ ...all, [item.id]: a }))}
            onSend={sendAnswers}
            busy={busy}
          />
        )}
        {item.kind === 'session' && (
          <section aria-label="Session waiting" className="flex flex-col gap-3">
            <p className="text-sm">
              {typeof item.body.sessionTitle === 'string' ? item.body.sessionTitle : 'This session'} waits on a permission
              prompt or a question. Answer it in the session; looking at it takes it out of the Inbox.
            </p>
            <div>
              <Button variant="primary" onClick={doPrimary}>Open session <Kbd>↵</Kbd></Button>
            </div>
          </section>
        )}
        {item.kind === 'approval' && <Approval item={item} onAnswered={() => { onSelect(after(items, item.id)); void refresh(); }} />}
        {item.kind === 'stalled' && (
          <section aria-label="Stalled step" className="flex flex-col gap-3">
            <p className="text-sm">
              {typeof item.body.why === 'string' ? item.body.why : 'The step stopped without an outcome.'} Nudge it to
              carry on, or take it over and pick the outcome yourself.
            </p>
            <Textarea aria-label="Nudge message (optional)" placeholder="Your own words for the nudge (optional)" value={nudge}
              onChange={(e) => setNudge(e.target.value)} rows={2} />
            <div className="flex flex-wrap items-center gap-2">
              <Button variant="primary" pending={busy} onClick={() => void answer({ choice: 'nudge', ...(nudge.trim() ? { text: nudge } : {}) })}>
                Nudge <Kbd>↵</Kbd>
              </Button>
              <Button variant="secondary" aria-expanded={takingOver} onClick={() => setTakingOver((t) => !t)}>Take over</Button>
            </div>
            {takingOver && <Outcomes item={item} busy={busy} onPick={(choice) => void answer({ choice })} />}
          </section>
        )}
        {item.kind === 'timed_out' && (
          <section aria-label="Timed out" className="flex flex-col gap-3">
            <p className="text-sm">
              {typeof item.body.node === 'string' ? `Step ${item.body.node}` : 'The step'}
              {typeof item.body.workflow === 'string' ? ` of workflow ${item.body.workflow}` : ''} ended without an
              outcome: {typeof item.body.why === 'string' ? item.body.why : 'it ran out of time'}. Resume runs it again on the
              same ticket and attempt{item.sessionId ? ', in the same session' : ''}; Stop leaves it stopped.
            </p>
            <Textarea aria-label="Note (optional)" placeholder="A note for the agent with Resume (optional)" value={nudge}
              onChange={(e) => setNudge(e.target.value)} rows={2} />
            <Choices item={item} busy={busy} onPick={(choice) => void answer({ choice, ...(choice === 'resume' && nudge.trim() ? { text: nudge } : {}) })} />
          </section>
        )}
        {item.kind === 'loop_limit' && (
          <section aria-label="Loop limit" className="flex flex-col gap-3">
            <p className="text-sm">This step has run as many times as its loop limit allows. Its passes so far:</p>
            <ol className="flex flex-col gap-2">
              {passesOf(item).map((p, n) => (
                <li key={n} className="rounded-md border border-border p-2 text-sm">
                  <span className="font-medium">{p.persona}</span>: {p.outcome ?? 'no outcome'}
                  {p.summary && <p className="mt-1 text-muted-foreground">{p.summary}</p>}
                </li>
              ))}
            </ol>
            <Choices item={item} busy={busy} onPick={(choice) => void answer({ choice })} />
          </section>
        )}
        {item.kind === 'manual' && (
          <section aria-label="Your turn" className="flex flex-col gap-3">
            <p className="text-sm">
              This stage is yours{typeof item.body.workflow === 'string' ? ` in workflow ${item.body.workflow}` : ''}: do it,
              then pick how it ended.
            </p>
            {typeof item.body.prompt === 'string' && (
              <div className="rounded-md border border-border p-3 text-sm"><Markdown>{item.body.prompt}</Markdown></div>
            )}
            <Textarea aria-label="What you did (optional)" placeholder="What you did or found (optional): the next step reads it"
              value={nudge} onChange={(e) => setNudge(e.target.value)} rows={2} />
            <div role="group" aria-label="Pick the outcome" className="flex flex-wrap items-center gap-2">
              {item.choices.map((c) => (
                <Button key={c.id} variant={c.primary ? 'primary' : 'secondary'} pending={busy && c.primary} disabled={busy}
                  onClick={() => void answer({ choice: c.id, ...(nudge.trim() ? { text: nudge } : {}) })}>
                  {c.title} {c.primary && <Kbd>↵</Kbd>}
                </Button>
              ))}
            </div>
          </section>
        )}
        {(item.kind === 'gate') && (
          <section aria-label="Gate" className="flex flex-col gap-3">
            {typeof item.body.workflow === 'string' && (
              <p className="text-meta text-muted-foreground">From workflow {item.body.workflow}</p>
            )}
            <Textarea
              aria-label="Note (optional)"
              placeholder={item.ticketId ? 'A note with your choice (optional), e.g. what to change when you reject it' : 'A note with your choice (optional)'}
              value={nudge}
              onChange={(e) => setNudge(e.target.value)}
              rows={3}
            />
            {item.ticketId && (
              <p className="text-meta text-muted-foreground">
                A note goes on the ticket as your comment, so the agent working on it gets it.
              </p>
            )}
            <Choices item={item} busy={busy} onPick={(choice) => void answer({ choice, ...(nudge.trim() ? { text: nudge } : {}) })} />
          </section>
        )}
      </div>
    </div>
  );
}

/** An Update: what it says, read once it is open, and a way to what it is about. */
export function UpdateView({ update, onOpenTarget }: { update: Notification; onOpenTarget?: (t: NotificationTarget) => void }) {
  const read = useReadNotification();
  const unread = update.readAt === null;
  const { mutate } = read;
  useEffect(() => {
    if (unread) mutate(update.id);
  }, [unread, update.id, mutate]);
  const open = update.target.kind !== 'none' && onOpenTarget ? () => onOpenTarget(update.target) : null;
  const meta = [update.severity === 'error' ? 'Error' : update.severity === 'warning' ? 'Warning' : null,
    relativeTime(update.createdAt), update.clearedAt !== null ? 'resolved' : null].filter(Boolean).join(' · ');
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <ViewHeader
        title={update.title}
        titleTooltip={update.title}
        meta={<span className="text-meta text-muted-foreground">{meta}</span>}
        actions={open ? <Button variant="ghost" size="sm" onClick={open}>Open</Button> : undefined}
      />
      <div className="flex min-h-0 flex-1 flex-col gap-4 overflow-auto p-4">
        {update.body && <p className="text-sm whitespace-pre-wrap">{update.body}</p>}
        {open && <div><Button variant="primary" onClick={open}>Open</Button></div>}
      </div>
    </div>
  );
}

/** A gate's or loop limit's choices, the primary one first and marked. */
function Choices({ item, busy, onPick }: { item: InboxItem; busy: boolean; onPick: (choice: string) => void }) {
  return (
    <div className="flex flex-wrap items-center gap-2">
      {item.choices.map((c) => (
        <Button key={c.id} variant={c.primary ? 'primary' : 'secondary'} pending={busy && c.primary} disabled={busy} onClick={() => onPick(c.id)}>
          {c.title} {c.primary && <Kbd>↵</Kbd>}
        </Button>
      ))}
    </div>
  );
}

/** Take over: the step's outcomes. */
function Outcomes({ item, busy, onPick }: { item: InboxItem; busy: boolean; onPick: (choice: string) => void }) {
  const outcomes = item.choices.filter((c) => c.id.startsWith('outcome:'));
  return (
    <div role="group" aria-label="Pick the outcome" className="flex flex-wrap items-center gap-2">
      <span className="text-sm text-muted-foreground">The step ends with:</span>
      {outcomes.map((c) => (
        <Button key={c.id} variant="secondary" disabled={busy} onClick={() => onPick(c.id)}>{c.id.slice('outcome:'.length)}</Button>
      ))}
    </div>
  );
}

function Questions({ item, answers, onChange, onSend, busy }: {
  item: InboxItem;
  answers: string[];
  onChange: (a: string[]) => void;
  onSend: () => void;
  busy: boolean;
}) {
  const qs = questionsOf(item);
  const set = (n: number, v: string) => onChange(qs.map((_, i) => (i === n ? v : answers[i] ?? '')));
  return (
    <section aria-label="Questions" className="flex flex-col gap-4">
      {qs.map((q, n) => (
        <div key={n} className="flex flex-col gap-1.5">
          <p className="text-sm font-medium"><Markdown>{q.question}</Markdown></p>
          {q.choices.length > 0 && (
            <div className="flex flex-wrap gap-1.5">
              {q.choices.map((c) => (
                <Button key={c} size="sm" variant="secondary" aria-pressed={answers[n] === c}
                  className={answers[n] === c ? 'bg-accent' : undefined} onClick={() => set(n, c)}>{c}</Button>
              ))}
            </div>
          )}
          <Textarea aria-label={`Answer ${n + 1}`} value={answers[n] ?? ''} onChange={(e) => set(n, e.target.value)} rows={2} />
        </div>
      ))}
      <div>
        <Button variant="primary" pending={busy} disabled={!answers.some((a) => a?.trim())} onClick={onSend}>Send answers</Button>
      </div>
    </section>
  );
}

/** Answers an approval item's permission request in its session; `onAnswered` once it went through. */
export async function answerApproval(item: InboxItem, a: ApprovalAnswer, onAnswered: () => void) {
  const requestId = typeof item.body.requestId === 'string' ? item.body.requestId : '';
  try {
    await api.answerRequest(item.sessionId!, requestId, a);
    onAnswered();
  } catch (err) {
    if (err instanceof ApiError && err.code === 'request_closed') toast.info('That request was already answered or has expired.');
    else toast.error(actionError(err));
  }
}

/** An agent's open permission request, answered in its session. */
function Approval({ item, onAnswered }: { item: InboxItem; onAnswered: () => void }) {
  const tool = typeof item.body.tool === 'string' ? item.body.tool : 'tool';
  const diff = typeof item.body.diff === 'string' ? item.body.diff : null;
  const description = typeof item.body.description === 'string' ? item.body.description : undefined;
  const questions = requestQuestions(item.body.input);
  const onAnswer = (a: ApprovalAnswer) => answerApproval(item, a, onAnswered);
  return (
    <ApprovalCard
      kind={questions.length > 0 ? 'question' : 'approval'}
      title={approvalTitle(tool)}
      subject={!diff ? item.title : undefined}
      preview={diff ? <DiffViewer diff={diff} /> : undefined}
      description={description}
      canAlways={item.choices.some((c) => c.id === 'allow_always')}
      questions={questions.length > 0 ? questions : undefined}
      keyboard
      onAnswer={onAnswer}
    />
  );
}

/** Ready to accept: the attempt's changes, from its ticket. */
/**
 * The ticket's work, next to its tab in the workspace that has it (its Start workspace): its coder session (to talk to
 * it), Open changes, Open PR (on GitHub), and Run… over the runs and tests its worktree's configuration offers.
 */
function TicketWork({ item }: { item: InboxItem }) {
  const openChanges = useOpenChanges();
  const tabs = useTicketTabs();
  const ticket = useTicket(item.ticketId);
  const t = ticket.data;
  const attempt = t?.attempts.find((x) => x.id === item.attemptId) ?? t?.attempts.find((x) => x.state === 'active');
  const worktreeId = attempt?.worktreeId ?? null;
  const projectId = item.projectId ?? '';
  const { choices } = useRunChoices(projectId, worktreeId);
  // The attempt's agent session (the Coder's): the newest one, running ones first.
  const mine = (t?.sessions ?? []).filter((x) => attempt && x.attemptId === attempt.id);
  const coder = mine.find((x) => x.status === 'running') ?? mine[0];
  const start = useStartRun(projectId, (id) => tabs.openRun({ id, projectId }));
  const pr = attempt?.pr ?? null;
  if (!item.ticketId || !item.projectId || (!worktreeId && !coder && !pr)) return null;
  const run = (choice: string) => {
    const tab = tabs.placementNear(item.ticketId!);
    if (!worktreeId) return;
    start.mutate({ worktreeId, ...parseChoice(choice), ...(tab ? { tab } : {}) }, { onError: (err) => toast.error(actionError(err)) });
  };
  return (
    <div role="group" aria-label="The ticket's work" className="flex flex-wrap items-center gap-2">
      {coder && (
        <Button variant="secondary" size="sm" title={coder.title}
          onClick={() => tabs.openNear({ kind: 'session', id: coder.id }, projectId, item.ticketId!)}>
          Open coder session
        </Button>
      )}
      {worktreeId && (
        <Button variant="secondary" size="sm" onClick={() => openChanges({ id: worktreeId, projectId }, { ticketId: item.ticketId! })}>
          Open changes
        </Button>
      )}
      {pr && (
        <Button variant="secondary" size="sm" title={`#${pr.number} ${pr.title}`} onClick={() => window.open(pr.url, '_blank', 'noopener')}>
          Open PR
        </Button>
      )}
      {worktreeId && choices.length > 0 && (
        <RunMenu label="Run the ticket's branch" choices={choices} value="" onChange={run} disabled={start.isPending} className="w-40" />
      )}
    </div>
  );
}
