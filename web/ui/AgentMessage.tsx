import type { ReactNode } from 'react';
import { cn } from './lib/utils';

/** The chart series an avatar takes its colour from, by name: the same name always gets the same colour. */
const SERIES = 6;

function seriesOf(name: string): number {
  let h = 0;
  for (const c of name) h = (h * 31 + c.charCodeAt(0)) >>> 0;
  return (h % SERIES) + 1;
}

/**
 * A round initial for a persona or an agent ("C" for Coder), coloured by its name. Decorative: the name is always
 * written beside it.
 */
export function Avatar({ name, size = 'md', className }: { name: string; size?: 'sm' | 'md'; className?: string }) {
  return (
    <span
      aria-hidden
      data-series={seriesOf(name)}
      style={{ backgroundColor: `var(--pdt-chart-${seriesOf(name)})` }}
      className={cn(
        'inline-grid shrink-0 place-items-center rounded-full font-bold text-background select-none',
        size === 'sm' ? 'size-4 text-2xs' : 'size-5 text-2xs',
        className,
      )}
    >
      {(name.trim().charAt(0) || '?').toUpperCase()}
    </span>
  );
}

/**
 * What an agent (or a workflow) said, as a card from it: its avatar and `label` ("Coder's context", "Its last
 * message") over the text. `name` is who said it.
 */
export function AgentMessage({ name, label, children, className, 'aria-label': ariaLabel }: {
  name: string;
  label?: ReactNode;
  children: ReactNode;
  className?: string;
  'aria-label'?: string;
}) {
  return (
    <div role="group" aria-label={ariaLabel ?? `From ${name}`} className={cn('overflow-hidden rounded-lg border border-border bg-card', className)}>
      <div className="flex items-center gap-2 border-b border-border px-3.5 py-2 text-xs text-muted-foreground">
        <Avatar name={name} />
        <span className="min-w-0 truncate">{label ?? name}</span>
      </div>
      <div className="px-3.5 py-3 text-sm break-words">{children}</div>
    </div>
  );
}

export interface PassRow {
  /** Who ran it: a persona's name. */
  name: string;
  /** "Pass 2 · Reviewer"; the name when left out. */
  title?: ReactNode;
  /** The outcome it gave, shown as a chip; null for none. */
  outcome?: string | null;
  /** The chip's tone: `good` (approved, pass), `bad` (changes requested, fail) or neutral. */
  tone?: 'good' | 'bad' | 'neutral';
  summary?: ReactNode;
}

const outcomeTone = { good: 'bg-running-soft text-running', bad: 'bg-failed-soft text-failed', neutral: 'bg-muted text-muted-foreground' };

/** Agent steps' passes, one row each: the persona's avatar, who and which pass, the outcome as a chip, and its summary. */
export function PassList({ passes, className, 'aria-label': ariaLabel = 'Passes' }: {
  passes: PassRow[];
  className?: string;
  'aria-label'?: string;
}) {
  return (
    <ol aria-label={ariaLabel} className={cn('flex flex-col overflow-hidden rounded-lg border border-border bg-card', className)}>
      {passes.map((p, n) => (
        <li key={n} className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-2.5 border-t border-border px-3.5 py-2.5 first:border-t-0">
          <Avatar name={p.name} className="mt-0.5" />
          <div className="flex min-w-0 flex-col gap-0.5">
            <div className="flex flex-wrap items-center gap-x-1.5 text-sm">
              <span className="font-semibold">{p.title ?? p.name}</span>
              <span className={cn('rounded-full px-1.5 text-meta font-semibold', outcomeTone[p.tone ?? 'neutral'])}>
                {p.outcome ?? 'no outcome'}
              </span>
            </div>
            {p.summary && <div className="text-sm break-words text-muted-foreground">{p.summary}</div>}
          </div>
        </li>
      ))}
    </ol>
  );
}
