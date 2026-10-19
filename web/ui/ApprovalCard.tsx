import { useEffect, useRef, useState, type KeyboardEvent as ReactKeyboardEvent, type ReactNode } from 'react';
import { ShieldQuestion } from 'lucide-react';
import { cn } from './lib/utils';
import { Button } from './Button';
import { Input } from './Input';
import { Kbd } from './Kbd';

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
  /** Offer "Always allow in this session". */
  canAlways?: boolean;
  /** `question` cards: what is asked. */
  questions?: ApprovalQuestion[];
  /** `plan` cards: the plan (usually rendered Markdown). */
  plan?: ReactNode;
  /** `plan` cards: also offer "Approve, accept edits" (the agent then edits without asking). */
  planAcceptEdits?: boolean;
  /** Called with the answer; the card shows it pending until the promise settles. Handle (toast) errors there. */
  onAnswer: (answer: ApprovalAnswer) => Promise<unknown> | void;
  /** `y` approves and `n` starts a denial, from anywhere on the page outside text fields. Give it to one card. */
  keyboard?: boolean;
  disabled?: boolean;
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
  canAlways = false,
  questions = [],
  plan,
  planAcceptEdits = false,
  onAnswer,
  keyboard = false,
  disabled = false,
  className,
}: ApprovalCardProps) {
  const [pending, setPending] = useState<string | null>(null);
  const [denying, setDenying] = useState(false);
  const [reason, setReason] = useState('');
  const [picked, setPicked] = useState<Record<string, string[]>>({});
  // Questions answered in the user's own words ("Other…"): question → text (present while the field is open).
  const [other, setOther] = useState<Record<string, string>>({});
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
  const approve = () =>
    void answer('approve', kind === 'plan' ? { type: 'plan', approve: true } : { type: 'approve', always: false });
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

  const toggle = (q: ApprovalQuestion, label: string) => {
    setPicked((p) => {
      const cur = p[q.question] ?? [];
      const next = q.multiSelect ? (cur.includes(label) ? cur.filter((l) => l !== label) : [...cur, label]) : [label];
      return { ...p, [q.question]: next };
    });
    // One answer per single-choice question: an option closes "Other…".
    if (!q.multiSelect) setOther(({ [q.question]: _, ...rest }) => rest);
  };
  const toggleOther = (q: ApprovalQuestion) => {
    const open = q.question in other;
    setOther(({ [q.question]: _, ...rest }) => (open ? rest : { ...rest, [q.question]: '' }));
    if (!open && !q.multiSelect) setPicked(({ [q.question]: _, ...rest }) => rest);
  };
  const answersOf = (): Record<string, string[]> =>
    Object.fromEntries(
      questions.map((q) => {
        const own = other[q.question]?.trim();
        return [q.question, [...(picked[q.question] ?? []), ...(own ? [own] : [])]];
      }),
    );
  const allAnswered = questions.length > 0 && Object.values(answersOf()).every((a) => a.length > 0);
  const hint = (k: string) => (keyboard ? <span aria-hidden className="ml-1 pointer-coarse:hidden"><Kbd>{k}</Kbd></span> : null);

  return (
    <section
      aria-label={title}
      className={cn('flex w-full flex-col gap-2 rounded-lg border border-waiting bg-card p-3 text-sm shadow-popover', className)}
    >
      <header className="flex items-center gap-2 font-medium">
        <ShieldQuestion aria-hidden className="size-4 shrink-0 text-waiting" />
        <span className="min-w-0">{title}</span>
      </header>
      {subject && (
        <pre className="max-h-48 overflow-auto rounded-md border border-border bg-surface-2 p-2 font-mono text-xs whitespace-pre-wrap break-words">
          {subject}
        </pre>
      )}
      {preview && <div className="max-h-96 overflow-auto">{preview}</div>}
      {description && <div className="text-xs text-muted-foreground">{description}</div>}
      {kind === 'plan' && plan && <div className="max-h-96 overflow-auto rounded-md border border-border p-2">{plan}</div>}
      {kind === 'question' &&
        questions.map((q) => (
          <fieldset key={q.question} className="flex flex-col gap-1.5">
            <legend className="mb-1 text-sm">
              {q.header && <span className="mr-1.5 text-2xs font-medium tracking-wide text-muted-foreground uppercase">{q.header}</span>}
              {q.question}
            </legend>
            <div className="flex flex-col gap-1">
              {q.options.map((o) => {
                const on = (picked[q.question] ?? []).includes(o.label);
                return (
                  <button
                    key={o.label}
                    type="button"
                    aria-pressed={on}
                    aria-label={o.label}
                    title={o.description}
                    disabled={busy}
                    onClick={() => toggle(q, o.label)}
                    className={cn(
                      'flex min-h-control-md w-full flex-wrap items-baseline gap-x-2 rounded-md border px-2 py-1.5 text-left text-xs outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-50 pointer-coarse:min-h-control-touch',
                      on ? 'border-primary bg-accent text-foreground' : 'border-border hover:bg-accent',
                    )}
                  >
                    <span className="font-medium">{o.label}</span>
                    {o.description && <span aria-hidden className="text-muted-foreground">{o.description}</span>}
                  </button>
                );
              })}
              <button
                type="button"
                aria-pressed={q.question in other}
                disabled={busy}
                onClick={() => toggleOther(q)}
                className={cn(
                  'flex min-h-control-md w-full items-center rounded-md border border-dashed px-2 text-left text-xs outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-50 pointer-coarse:min-h-control-touch',
                  q.question in other ? 'border-primary bg-accent text-foreground' : 'border-border text-muted-foreground hover:bg-accent',
                )}
              >
                Other…
              </button>
            </div>
            {q.question in other && (
              <Input
                aria-label={questions.length > 1 ? `Your answer: ${q.question}` : 'Your answer'}
                placeholder="Your answer"
                value={other[q.question]}
                disabled={busy}
                onChange={(e) => setOther((o) => ({ ...o, [q.question]: e.target.value }))}
                onKeyDown={(e) => {
                  if (e.key === 'Enter' && !e.nativeEvent.isComposing && allAnswered) {
                    e.preventDefault();
                    void answer('question', { type: 'question', answers: answersOf() });
                  }
                }}
              />
            )}
          </fieldset>
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
        <div className="flex flex-wrap items-center gap-2">
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
                  pending={pending === 'accept_edits'}
                  disabled={busy}
                  onClick={() => void answer('accept_edits', { type: 'plan', approve: true, mode: 'accept_edits' })}
                >
                  Approve, accept edits
                </Button>
              )}
              <Button variant={kind === 'plan' && planAcceptEdits ? 'secondary' : 'primary'} pending={pending === 'approve'} disabled={busy} onClick={approve}>
                {kind === 'plan' ? (planAcceptEdits ? 'Approve, ask before edits' : 'Approve plan') : 'Approve'}{hint('y')}
              </Button>
              <Button variant="secondary" disabled={busy} onClick={() => setDenying(true)}>
                {kind === 'plan' ? 'Keep planning' : 'Deny'}{hint('n')}
              </Button>
              {canAlways && kind === 'approval' && (
                <Button variant="ghost" pending={pending === 'always'} disabled={busy} onClick={() => void answer('always', { type: 'approve', always: true })}>
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
