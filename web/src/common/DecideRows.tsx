// The PM's batch of Inbox decisions (PDT-475): one row per item with its ticket, the decision and why; the user
// unticks the rows they don't want before Allow, in the Inbox or on the PM conversation's card (PDT-502).
import { Fragment, useState, type ReactNode } from 'react';
import { CheckTable, Chip, type CheckTableColumn } from '@ui';
import type { DecideChoice } from '../api/gen/DecideChoice';
import type { DecideRow } from '../api/gen/DecideRow';
import { PRIORITY_LABEL } from './PriorityIcon';
import { TicketPill } from './TicketPill';
import { isTicketKey } from '../tickets/model';

export const DECISION_LABEL: Record<DecideChoice, string> = {
  accept_todo: 'Accept to Todo',
  accept_backlog: 'Accept to backlog',
  duplicate: 'Duplicate',
  reject: 'Reject',
};

/** What a row's decision does beyond its choice: the edits, `agentic` and the ticket it repeats. */
export function decisionDetails(r: DecideRow): ReactNode[] {
  const out: ReactNode[] = [];
  if (r.duplicateOf) out.push(<>of {isTicketKey(r.duplicateOf) ? <TicketPill ticketKey={r.duplicateOf.toUpperCase()} /> : <Chip>{r.duplicateOf}</Chip>}</>);
  if (r.agentic) out.push('agentic');
  if (r.title) out.push(`title “${r.title}”`);
  if (r.priority) out.push(`${PRIORITY_LABEL[r.priority]} priority`);
  if (r.labels) out.push(r.labels.length ? `labels ${r.labels.join(', ')}` : 'no labels');
  return out.map((d, n) => <Fragment key={n}>{n > 0 && ' · '}{d}</Fragment>);
}

const COLUMNS: CheckTableColumn<DecideRow>[] = [
  { id: 'ticket', header: 'Ticket', cell: (r) => <><TicketPill ticketKey={r.ticketKey} /> {r.ticketTitle}</> },
  {
    id: 'decision',
    header: 'Decision',
    cell: (r) => (
      <>
        {DECISION_LABEL[r.decision]}
        {decisionDetails(r).length > 0 && <div className="text-xs text-muted-foreground">{decisionDetails(r)}</div>}
        {r.note && <div className="mt-1 text-xs sm:hidden">{r.note}</div>}
      </>
    ),
  },
  { id: 'note', header: 'Note', wide: true, cell: (r) => r.note ?? '' },
];

export function DecideRows({ rows, kept, onToggle, disabled }: {
  rows: DecideRow[];
  /** Whether row `n` is ticked. */
  kept: (n: number) => boolean;
  onToggle: (n: number, on: boolean) => void;
  disabled?: boolean;
}) {
  return (
    <CheckTable aria-label="Decisions" rows={rows} rowKey={(r) => r.itemId} columns={COLUMNS} checked={kept} onCheck={onToggle}
      checkLabel={(r) => `Keep ${r.ticketKey}`} disabled={disabled} />
  );
}

/** Which of `count` rows are ticked (all to start with): `kept` lists them, 0-based, for the Allow. */
function useKeptRows(count: number) {
  const [unticked, setUnticked] = useState<ReadonlySet<number>>(() => new Set());
  const isKept = (n: number) => !unticked.has(n);
  const toggle = (n: number, on: boolean) => setUnticked((u) => {
    const next = new Set(u);
    if (on) next.delete(n); else next.add(n);
    return next;
  });
  const kept = Array.from({ length: count }, (_, n) => n).filter(isKept);
  return { kept, isKept, toggle };
}

/**
 * The decision on a batch of rows, wherever it is asked (the conversation's card, the Inbox): which rows are ticked,
 * the copy, and the `y`/`n` keys. `allow` gets the rows kept.
 */
export function useRowsDecision(rows: DecideRow[] | null) {
  const count = rows?.length ?? 0;
  const { kept, isKept, toggle } = useKeptRows(count);
  return {
    kept,
    isKept,
    toggle,
    title: count === 1 ? 'Make this decision?' : `Make these ${count} decisions?`,
    /** What the proposal is, for the Inbox's line: "PM proposes 3 decisions on your Inbox." */
    proposes: (who: string) => `${who} proposes ${count === 1 ? 'a decision' : `${count} decisions`} on your Inbox.`,
    help: "Untick any you don't want. Allow applies the rest as your own decisions; Deny changes nothing.",
    allowTitle: kept.length === count ? 'Allow' : `Allow ${kept.length} of ${count}`,
    canAllow: kept.length > 0,
    /** How many are ticked; announce it (a live region) as it changes. */
    ticked: `${kept.length} of ${count} ticked`,
    /** `y` allows the rows kept (when any are), `n` denies: true when the key was one of them. */
    onKey: (key: string, allow: (kept: number[]) => void, deny: () => void) => {
      if (key === 'y') { if (kept.length > 0) allow(kept); }
      else if (key === 'n') deny();
      else return false;
      return true;
    },
  };
}
