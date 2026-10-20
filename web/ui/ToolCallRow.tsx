import { useId, useState, type ReactNode } from 'react';
import { Ban, Check, ChevronRight, X } from 'lucide-react';
import { cn } from './lib/utils';
import { Spinner } from './Spinner';

export type ToolCallStatus = 'running' | 'done' | 'failed' | 'declined';

const STATUS_LABEL: Record<ToolCallStatus, string> = { running: 'Running', done: 'Done', failed: 'Failed', declined: 'Declined' };

/** The status icon of a tool call: a spinner, ✓, ✗ (the only coloured one) or a "declined" mark, each named for screen readers. */
export function ToolStatusIcon({ status, className }: { status: ToolCallStatus; className?: string }) {
  if (status === 'running') return <Spinner label={STATUS_LABEL.running} className={cn('size-3.5 text-muted-foreground', className)} />;
  const Icon = status === 'done' ? Check : status === 'failed' ? X : Ban;
  return (
    <span role="img" aria-label={STATUS_LABEL[status]} className={cn('inline-flex shrink-0', className)}>
      <Icon
        aria-hidden
        className={cn('size-3.5', status === 'failed' ? 'text-failed' : 'text-muted-foreground')}
      />
    </span>
  );
}

export interface ToolCallRowProps {
  /** What the tool does, as a verb phrase ("Run", "Edit", "Search"). */
  verb: ReactNode;
  /** Its subject (a command, a file, a pattern), shown in code; omitted when the tool has none. */
  subject?: string;
  /** Omit for a row that is a summary, not a call (e.g. "Ran 3 commands"). */
  status?: ToolCallStatus;
  /** Right-aligned details: a duration, an exit code, "+12 −3". */
  meta?: ReactNode;
  /** Shown when the row is expanded (input, output); the row is a button only when there is something to show. */
  details?: ReactNode;
  /** Nested rows (sub-agent calls), shown when expanded, indented under this one. */
  children?: ReactNode;
  defaultOpen?: boolean;
  className?: string;
}

/**
 * One line per tool call in a conversation ("Run `cargo test`  ✓ 12 s"), expanding to its input and output. Also
 * the collapsed summary of several calls (no `status`, children are the calls).
 */
export function ToolCallRow({ verb, subject, status, meta, details, children, defaultOpen = false, className }: ToolCallRowProps) {
  const [open, setOpen] = useState(defaultOpen);
  const id = useId();
  const expandable = !!details || !!children;
  const line = (
    <>
      {expandable ? (
        <ChevronRight aria-hidden className={cn('size-3.5 shrink-0 text-muted-foreground transition-transform motion-reduce:transition-none', open && 'rotate-90')} />
      ) : (
        <span aria-hidden className="size-3.5 shrink-0" />
      )}
      <span className="flex min-w-0 flex-1 items-baseline gap-1.5">
        <span className="shrink-0 text-muted-foreground">{verb}</span>{' '}
        {subject && (
          <code className="min-w-0 truncate rounded-sm bg-muted/60 px-1 py-px font-mono text-meta text-muted-foreground group-hover:text-foreground">{subject}</code>
        )}
      </span>
      {meta && ' '}
      {meta && <span className="shrink-0 text-meta text-muted-foreground tabular-nums">{meta}</span>}
      {status && <ToolStatusIcon status={status} />}
    </>
  );
  // Quiet next to the conversation's text (PDT-75): muted until hovered; only a failure is coloured.
  const rowClass = 'group flex w-full min-w-0 items-center gap-1.5 rounded-md px-1 py-0.5 text-left text-xs';
  return (
    <div className={cn('flex flex-col', className)} data-status={status}>
      {expandable ? (
        <button
          type="button"
          aria-expanded={open}
          aria-controls={id}
          onClick={() => setOpen((o) => !o)}
          className={cn(rowClass, 'outline-none hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring pointer-coarse:min-h-control-touch')}
        >
          {line}
        </button>
      ) : (
        <div className={rowClass}>{line}</div>
      )}
      {open && expandable && (
        <div id={id} className="flex flex-col gap-1.5 pt-1 pb-1.5 pl-5">
          {details}
          {children && <div className="flex flex-col border-l border-border pl-2">{children}</div>}
        </div>
      )}
    </div>
  );
}

// ESC [ … final byte (colours, cursor moves) and ESC ] … BEL/ST (titles, links): output is shown as plain text.
const ANSI = new RegExp(String.raw`\u001b(?:\[[0-?]*[ -/]*[@-~]|\][^\u0007\u001b]*(?:\u0007|\u001b\\)|[@-Z\\-_])`, 'g');

/** `text` without terminal escape sequences. */
export const stripAnsi = (text: string) => text.replace(ANSI, '');

/**
 * A terminal-styled block for a tool's input or output: mono, scrolls past ~20 lines, escape sequences removed.
 * `label` names it ("Input", "Output").
 */
export function ToolOutput({ label, children, tone = 'default', className }: {
  label?: string;
  children: string;
  /** `error`: a failed call's output. */
  tone?: 'default' | 'error';
  className?: string;
}) {
  return (
    <figure className={cn('flex min-w-0 flex-col gap-0.5', className)}>
      {label && <figcaption className="text-2xs font-medium tracking-wide text-muted-foreground uppercase">{label}</figcaption>}
      <pre
        className={cn(
          'max-h-80 overflow-auto rounded-md border border-border bg-surface-2 p-2 font-mono text-meta leading-relaxed whitespace-pre-wrap break-words text-foreground',
          tone === 'error' && 'text-failed',
        )}
      >
        {stripAnsi(children)}
      </pre>
    </figure>
  );
}
