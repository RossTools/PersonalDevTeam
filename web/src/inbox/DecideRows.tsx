// The PM's batch of Inbox decisions (PDT-475): one row per item with its ticket, the decision and why; the user
// unticks the rows they don't want before Allow.
import { Checkbox } from '@ui';
import type { DecideChoice } from '../api/gen/DecideChoice';
import type { DecideRow } from '../api/gen/DecideRow';
import { PRIORITY_LABEL } from '../common/PriorityIcon';

export const DECISION_LABEL: Record<DecideChoice, string> = {
  accept_todo: 'Accept to Todo',
  accept_backlog: 'Accept to backlog',
  duplicate: 'Duplicate',
  reject: 'Reject',
};

/** What a row's decision does beyond its choice: the edits, `agentic` and the ticket it repeats. */
function details(r: DecideRow): string {
  const out: string[] = [];
  if (r.duplicateOf) out.push(`of ${r.duplicateOf}`);
  if (r.agentic) out.push('agentic');
  if (r.title) out.push(`title “${r.title}”`);
  if (r.priority) out.push(`${PRIORITY_LABEL[r.priority]} priority`);
  if (r.labels) out.push(r.labels.length ? `labels ${r.labels.join(', ')}` : 'no labels');
  return out.join(' · ');
}

export function DecideRows({ rows, kept, onToggle, disabled }: {
  rows: DecideRow[];
  /** Whether row `n` is ticked. */
  kept: (n: number) => boolean;
  onToggle: (n: number, on: boolean) => void;
  disabled?: boolean;
}) {
  return (
    <div className="overflow-x-auto rounded-lg border border-border bg-card">
      <table className="w-full text-sm" aria-label="Decisions">
        <thead>
          <tr className="text-left text-xs text-muted-foreground">
            <th className="w-8 px-3 py-2 font-medium"><span className="sr-only">Keep</span></th>
            <th className="px-3 py-2 font-medium">Ticket</th>
            <th className="px-3 py-2 font-medium">Decision</th>
            <th className="px-3 py-2 font-medium max-sm:hidden">Note</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((r, n) => (
            <tr key={r.itemId} className={kept(n) ? 'border-t border-border' : 'border-t border-border text-muted-foreground line-through'}>
              <td className="px-3 py-2 align-top">
                <Checkbox aria-label={`Keep ${r.ticketKey}`} checked={kept(n)} disabled={disabled} onChange={(on) => onToggle(n, on)} />
              </td>
              <td className="px-3 py-2 align-top">
                <span className="font-mono text-xs">{r.ticketKey}</span> {r.ticketTitle}
              </td>
              <td className="px-3 py-2 align-top">
                {DECISION_LABEL[r.decision]}
                {details(r) && <div className="text-xs text-muted-foreground">{details(r)}</div>}
              </td>
              <td className="px-3 py-2 align-top max-sm:hidden">{r.note ?? ''}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
