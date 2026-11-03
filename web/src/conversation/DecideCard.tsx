// The PM's `inbox_decide` asked in its conversation (PDT-502): the table of its decisions, where the user unticks the
// rows they don't want, then allows the rest or denies the call. The same request is the Inbox's approval item.
import { ApprovalCard, type ApprovalAnswer } from '@ui';
import type { DecideRow } from '../api/gen/DecideRow';
import { allowRowsTitle, DecideRows, useKeptRows } from '../common/DecideRows';

export function DecideCard({ id, rows, keyboard, onAnswer }: {
  /** The card's DOM id. */
  id: string;
  rows: DecideRow[];
  keyboard: boolean;
  /** `kept`: the rows an Allow keeps. */
  onAnswer: (answer: ApprovalAnswer, kept?: number[]) => Promise<unknown>;
}) {
  const { kept, isKept, toggle } = useKeptRows(rows.length);
  return (
    <ApprovalCard
      id={id}
      title={rows.length === 1 ? 'Make this decision?' : `Make these ${rows.length} decisions?`}
      preview={<DecideRows rows={rows} kept={isKept} onToggle={toggle} />}
      description="Untick any you don't want. Allow applies the rest as your own decisions; Deny changes nothing."
      approveLabel={allowRowsTitle(kept.length, rows.length)}
      approveDisabled={kept.length === 0}
      keyboard={keyboard}
      onAnswer={(a) => onAnswer(a, a.type === 'approve' ? kept : undefined)}
    />
  );
}
