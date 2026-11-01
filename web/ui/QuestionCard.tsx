import { useRef, type KeyboardEvent, type ReactNode } from 'react';
import { Textarea } from './Textarea';
import { cn } from './lib/utils';

export interface QuestionChoice {
  label: string;
  /** A line under the label, when the asker gave one. */
  description?: string;
}

/** A question's answer: a choice's label, or (`other` not null) the user's own words. */
export interface QuestionAnswer {
  choice: string | null;
  other: string | null;
}

export const NO_ANSWER: QuestionAnswer = { choice: null, other: null };

/** The text an answer sends: the user's own words when they chose Something else, else the choice. */
export function answerText(a: QuestionAnswer | undefined): string {
  if (!a) return '';
  return a.other !== null ? a.other : a.choice ?? '';
}

/** The input id of a card's Something else (`${id}-other`), for focusing it from a key. */
export const otherInputId = (id: string) => `${id}-other`;

const option = cn(
  'grid w-full grid-cols-[auto_minmax(0,1fr)_auto] items-start gap-x-2.5 rounded-lg border px-3 py-2 text-left text-sm outline-none',
  'focus-within:ring-2 focus-within:ring-ring pointer-coarse:min-h-control-touch',
);

function Radio({ on }: { on: boolean }) {
  return (
    <span aria-hidden className={cn('mt-0.5 grid size-4 place-items-center rounded-full border-[1.5px]', on ? 'border-primary' : 'border-muted-foreground')}>
      {on && <span className="size-2 rounded-full bg-primary" />}
    </span>
  );
}

function Num({ n, show }: { n: number; show: boolean }) {
  if (!show) return <span />;
  return (
    <span aria-hidden className="rounded-sm border border-border px-1 font-mono text-2xs text-muted-foreground pointer-coarse:hidden">{n}</span>
  );
}

/**
 * One of an agent's questions, as a card: "1 / 2" and the question, then its choices as radio options (each with its
 * number key when `numbers`) and a last "Something else…" option to type an own answer. A question without choices
 * is a text box. The parent keeps the answer and handles the number keys (`otherInputId` finds the text box).
 */
export function QuestionCard({ id, index, total, question, label, choices, value, onChange, numbers = false, disabled, className }: {
  id: string;
  /** 0-based. */
  index: number;
  total: number;
  question: ReactNode;
  /** The question as text: the options' group is named by it. */
  label: string;
  choices: QuestionChoice[];
  value: QuestionAnswer;
  onChange: (a: QuestionAnswer) => void;
  /** Shows each option's number key (1–9). */
  numbers?: boolean;
  disabled?: boolean;
  className?: string;
}) {
  const group = useRef<HTMLDivElement>(null);
  const otherOn = value.other !== null;
  const count = choices.length + 1;
  // Up/Down move between the options, as in any radio group.
  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.key !== 'ArrowDown' && e.key !== 'ArrowUp') return;
    const radios = [...(group.current?.querySelectorAll<HTMLElement>('[role=radio]') ?? [])];
    const at = radios.indexOf(document.activeElement as HTMLElement);
    if (at < 0) return;
    e.preventDefault();
    const next = radios[(at + (e.key === 'ArrowDown' ? 1 : count - 1)) % count];
    next.focus();
    next.click();
  };
  return (
    <section aria-label={`Question ${index + 1} of ${total}`} className={cn('rounded-lg border border-border bg-card', className)}>
      <div className="flex items-baseline gap-2.5 px-3.5 pt-3 pb-1">
        {total > 1 && <span className="shrink-0 text-meta font-semibold text-muted-foreground tabular-nums">{index + 1} / {total}</span>}
        <div className="min-w-0 text-sm font-semibold">{question}</div>
      </div>
      {choices.length === 0 ? (
        <div className="px-3.5 pt-1 pb-3.5">
          <Textarea id={otherInputId(id)} aria-label={`Answer ${index + 1}`} placeholder="Your answer" rows={2} disabled={disabled}
            value={value.other ?? ''} onChange={(e) => onChange({ choice: null, other: e.target.value })} />
        </div>
      ) : (
        <div ref={group} role="radiogroup" aria-label={label} onKeyDown={onKeyDown} className="flex flex-col gap-1.5 px-3.5 pt-2 pb-3.5">
          {choices.map((c, n) => {
            const on = !otherOn && value.choice === c.label;
            return (
              <button
                key={c.label}
                type="button"
                role="radio"
                aria-checked={on}
                tabIndex={on || (n === 0 && !otherOn && !choices.some((x) => x.label === value.choice)) ? 0 : -1}
                disabled={disabled}
                onClick={() => onChange({ choice: c.label, other: null })}
                className={cn(option, on ? 'border-primary bg-accent' : 'border-border bg-background hover:border-muted-foreground')}
              >
                <Radio on={on} />
                <span className="flex min-w-0 flex-col">
                  <span className="font-medium break-words">{c.label}</span>
                  {c.description && <span className="text-xs text-muted-foreground">{c.description}</span>}
                </span>
                <Num n={n + 1} show={numbers && n < 9} />
              </button>
            );
          })}
          <div className={cn(option, otherOn ? 'border-primary bg-accent' : 'border-border bg-background hover:border-muted-foreground')}>
            <button
              type="button"
              role="radio"
              aria-checked={otherOn}
              aria-label="Something else"
              tabIndex={otherOn ? 0 : -1}
              disabled={disabled}
              onClick={() => {
                onChange({ choice: value.choice, other: value.other ?? '' });
                document.getElementById(otherInputId(id))?.focus();
              }}
              className="rounded-full outline-none"
            >
              <Radio on={otherOn} />
            </button>
            <input
              id={otherInputId(id)}
              aria-label={`Something else for question ${index + 1}`}
              placeholder="Something else… (type your own answer)"
              disabled={disabled}
              value={value.other ?? ''}
              onFocus={() => !otherOn && onChange({ choice: value.choice, other: '' })}
              onChange={(e) => onChange({ choice: value.choice, other: e.target.value })}
              className="min-w-0 bg-transparent text-sm outline-none placeholder:text-muted-foreground"
            />
            <Num n={count} show={numbers && count <= 9} />
          </div>
        </div>
      )}
    </section>
  );
}
