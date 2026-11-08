import { useEffect, useId, useRef, useState, type KeyboardEvent as ReactKeyboardEvent, type ReactNode } from 'react';
import { ShieldQuestion } from 'lucide-react';
import { cn } from './lib/utils';
import { Button } from './Button';
import { Disclosure } from './Disclosure';
import { Input } from './Input';
import { Kbd } from './Kbd';
import { PreviewBox } from './PreviewBox';
import { NO_ANSWER, QuestionCard, type QuestionAnswer } from './QuestionCard';
import { SectionLabel } from './SectionLabel';

/** The answer the card gives (the same shape as the API's `Answer`). */
export type ApprovalAnswer =
  | { type: 'approve'; always: boolean }
  | { type: 'deny'; message?: string }
  | { type: 'question'; answers: Record<string, string[]> }
  | { type: 'plan'; approve: boolean; feedback?: string; mode?: 'accept_edits' };

export interface ApprovalQuestion {
  question: string;
  /** A short tag for the question ("Library"). */
  header?: string;
  options: { label: string; description?: string }[];
  /** Several options may be picked. */
  multiSelect?: boolean;
}

export interface ApprovalCardProps {
  /** `approval` (a tool wants to run), `question` (the agent asks), `plan` (the agent wants to leave plan mode). */
  kind?: 'approval' | 'question' | 'plan';
  /** "Run this command?", "Edit this file?" */
  title: string;
  /** The command, the file, the URL: shown in a code block. */
  subject?: string;
  /** What the request would change (a `DiffViewer`), under the subject. */
  preview?: ReactNode;
  description?: ReactNode;
  /** Everything the request carries (a tool's arguments), behind a ▾ under the subject. */
  details?: ReactNode;
  /** Names the ▾ (default "All arguments"). */
  detailsLabel?: string;
  /** Show `details` from the start (a request with nothing else to say what it does). */
  detailsOpen?: boolean;
  /** Offer "Always allow in this session". */
  canAlways?: boolean;
  /** `question` cards: what is asked. */
  questions?: ApprovalQuestion[];
  /** `plan` cards: the plan (usually rendered Markdown). */
  plan?: ReactNode;
  /** `plan` cards: also offer "Approve, accept edits" (the agent then edits without asking). */
  planAcceptEdits?: boolean;
  /** `approval` cards: the Allow button's text ("Allow 2 of 3"), and whether it is off (nothing left to allow). */
  approveLabel?: string;
  approveDisabled?: boolean;
  /** Called with the answer; the card shows it pending until the promise settles. Handle (toast) errors there. */
  onAnswer: (answer: ApprovalAnswer) => Promise<unknown> | void;
  /** `y` approves and `n` starts a denial, from anywhere on the page outside text fields. Give it to one card. */
  keyboard?: boolean;
  disabled?: boolean;
  /** The card's DOM id (something elsewhere jumps to it). */
  id?: string;
  className?: string;
}

/** Keys typed into a field, or with an overlay or menu open, are not ours. */
function isForeign(e: KeyboardEvent): boolean {
  if (e.defaultPrevented || e.ctrlKey || e.metaKey || e.altKey || e.repeat) return true;
  const t = e.target as HTMLElement | null;
  if (!t || !(t instanceof HTMLElement)) return false;
  if (t instanceof HTMLInputElement || t instanceof HTMLTextAreaElement || t instanceof HTMLSelectElement || t.isContentEditable) return true;
  return !!t.closest('[role=dialog],[role=menu],[role=listbox],.xterm');
}

/**
 * An agent's request in a conversation: approve or deny a tool call (with an optional reason, or "always" for the
 * session), answer its questions, or approve its plan. Presentational: the caller sends the answer.
 */
export function ApprovalCard({
  kind = 'approval',
  title,
  subject,
  preview,
  description,
  details,
  detailsLabel = 'All arguments',
  detailsOpen = false,
  canAlways = false,
  questions = [],
  plan,
  planAcceptEdits = false,
  approveLabel = 'Allow',
  approveDisabled = false,
  onAnswer,
  keyboard = false,
  disabled = false,
  id,
  className,
}: ApprovalCardProps) {
  const [pending, setPending] = useState<string | null>(null);
  const [denying, setDenying] = useState(false);
  const [reason, setReason] = useState('');
  // Each question's answer, by its text: the choices picked and the user's own words.
  const [picked, setPicked] = useState<Record<string, QuestionAnswer>>({});
  const idBase = useId();
  const reasonRef = useRef<HTMLInputElement>(null);
  const busy = disabled || pending !== null;

  const answer = async (key: string, a: ApprovalAnswer) => {
    if (busy) return;
    setPending(key);
    try {
      await onAnswer(a);
    } catch {
      // The caller reports it; the card stays answerable.
    } finally {
      setPending(null);
    }
  };
  const approve = () => {
    if (kind === 'approval' && approveDisabled) return;
    void answer('approve', kind === 'plan' ? { type: 'plan', approve: true } : { type: 'approve', always: false });
  };
  const deny = () => {
    const text = reason.trim();
    void answer(
      'deny',
      kind === 'plan'
        ? { type: 'plan', approve: false, ...(text ? { feedback: text } : {}) }
        : { type: 'deny', ...(text ? { message: text } : {}) },
    );
  };

  // Focus the reason when the deny form opens (by click or `n`); never from a ref callback (the UI re-renders often).
  useEffect(() => {
    if (denying) reasonRef.current?.focus();
  }, [denying]);

  const keys = useRef<(e: KeyboardEvent) => void>(() => {});
  keys.current = (e) => {
    if (busy || isForeign(e)) return;
    if (e.key === 'y' && kind !== 'question') {
      e.preventDefault();
      approve();
    } else if (e.key === 'n') {
      e.preventDefault();
      if (kind === 'question') void answer('deny', { type: 'deny' });
      else setDenying(true);
    }
  };
  useEffect(() => {
    if (!keyboard) return;
    const listener = (e: KeyboardEvent) => keys.current(e);
    window.addEventListener('keydown', listener);
    return () => window.removeEventListener('keydown', listener);
  }, [keyboard]);

  const onReasonKey = (e: ReactKeyboardEvent<HTMLInputElement>) => {
    if (e.key === 'Enter' && !e.nativeEvent.isComposing) {
      e.preventDefault();
      deny();
    } else if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      setDenying(false);
    }
  };

  const answersOf = (): Record<string, string[]> =>
    Object.fromEntries(
      questions.map((q) => {
        const a = picked[q.question] ?? NO_ANSWER;
        const own = a.other?.trim();
        if (q.multiSelect) return [q.question, [...(a.choices ?? []), ...(own ? [own] : [])]];
        return [q.question, a.other !== null ? (own ? [own] : []) : a.choice ? [a.choice] : []];
      }),
    );
  const allAnswered = questions.length > 0 && Object.values(answersOf()).every((a) => a.length > 0);
  const hint = (k: string) => (keyboard ? <span aria-hidden className="ml-1 pointer-coarse:hidden"><Kbd>{k}</Kbd></span> : null);

  return (
    <section
      id={id}
      tabIndex={id ? -1 : undefined}
      aria-label={title}
      className={cn('flex w-full scroll-mt-4 flex-col gap-2 rounded-lg border border-waiting bg-card p-3 text-sm shadow-popover outline-none focus-visible:ring-2 focus-visible:ring-ring', className)}
    >
      <header className="flex items-center gap-2 font-medium">
        <ShieldQuestion aria-hidden className="size-4 shrink-0 text-waiting" />
        <span className="min-w-0">{title}</span>
      </header>
      {subject && (
        <PreviewBox as="pre" className="max-h-48 overflow-auto p-2 font-mono text-xs whitespace-pre-wrap break-words">
          {subject}
        </PreviewBox>
      )}
      {preview && <div className="max-h-96 overflow-auto">{preview}</div>}
      {details && (
        <Disclosure title={detailsLabel} defaultOpen={detailsOpen} triggerClassName="pointer-coarse:min-h-control-touch" panelClassName="max-h-80 overflow-auto">
          {details}
        </Disclosure>
      )}
      {description && <div className="text-xs text-muted-foreground">{description}</div>}
      {kind === 'plan' && plan && <div className="max-h-96 overflow-auto rounded-md border border-border p-2">{plan}</div>}
      {kind === 'question' &&
        questions.map((q, n) => (
          <QuestionCard
            key={q.question}
            id={`${idBase}-q${n}`}
            index={n}
            total={questions.length}
            label={q.question}
            question={<>{q.header && <SectionLabel as="span" className="mr-1.5">{q.header}</SectionLabel>}{q.question}</>}
            choices={q.options}
            multiple={q.multiSelect}
            value={picked[q.question] ?? NO_ANSWER}
            disabled={busy}
            onChange={(a) => setPicked((p) => ({ ...p, [q.question]: a }))}
            onSubmit={() => {
              if (allAnswered) void answer('question', { type: 'question', answers: answersOf() });
            }}
          />
        ))}

      {denying && kind !== 'question' ? (
        <div className="flex flex-col gap-2 sm:flex-row sm:items-center">
          <Input
            ref={reasonRef}
            aria-label={kind === 'plan' ? 'What to change' : 'Tell it why'}
            placeholder={kind === 'plan' ? 'What to change (optional)' : 'Tell it why (optional)'}
            value={reason}
            disabled={busy}
            onChange={(e) => setReason(e.target.value)}
            onKeyDown={onReasonKey}
            className="flex-1"
          />
          <div className="flex gap-2">
            <Button variant={kind === 'plan' ? 'primary' : 'danger'} pending={pending === 'deny'} disabled={busy} onClick={deny}>
              {kind === 'plan' ? 'Keep planning' : 'Deny'}
            </Button>
            <Button variant="ghost" disabled={busy} onClick={() => setDenying(false)}>Cancel</Button>
          </div>
        </div>
      ) : (
        // On a phone the answers sit at the card's foot as big buttons, Deny on the left and Allow on the right.
        <div className="flex flex-wrap items-center gap-2 max-md:grid max-md:grid-cols-2 max-md:[&_button]:min-h-control-touch">
          {kind === 'question' ? (
            <>
              <Button variant="primary" pending={pending === 'question'} disabled={busy || !allAnswered} onClick={() => void answer('question', { type: 'question', answers: answersOf() })}>
                Submit
              </Button>
              <Button variant="ghost" pending={pending === 'deny'} disabled={busy} onClick={() => void answer('deny', { type: 'deny' })}>
                Dismiss{hint('n')}
              </Button>
            </>
          ) : (
            <>
              {kind === 'plan' && planAcceptEdits && (
                <Button
                  variant="primary"
                  className="max-md:col-span-2"
                  pending={pending === 'accept_edits'}
                  disabled={busy}
                  onClick={() => void answer('accept_edits', { type: 'plan', approve: true, mode: 'accept_edits' })}
                >
                  Approve, accept edits
                </Button>
              )}
              <Button
                variant={kind === 'plan' && planAcceptEdits ? 'secondary' : 'primary'}
                pending={pending === 'approve'}
                disabled={busy || (kind === 'approval' && approveDisabled)}
                onClick={approve}
                className={cn('max-md:order-2', kind === 'plan' && 'max-md:col-span-2')}
              >
                {kind === 'plan' ? (planAcceptEdits ? 'Approve, ask before edits' : 'Approve plan') : approveLabel}{hint('y')}
              </Button>
              <Button variant="secondary" disabled={busy} onClick={() => setDenying(true)} className={cn('max-md:order-1', kind === 'plan' && 'max-md:col-span-2')}>
                {kind === 'plan' ? 'Keep planning' : 'Deny'}{hint('n')}
              </Button>
              {canAlways && kind === 'approval' && (
                <Button variant="ghost" pending={pending === 'always'} disabled={busy} onClick={() => void answer('always', { type: 'approve', always: true })} className="max-md:order-3 max-md:col-span-2">
                  Always allow in this session
                </Button>
              )}
            </>
          )}
        </div>
      )}
    </section>
  );
}
