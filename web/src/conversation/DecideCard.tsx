// The PM's `inbox_decide` asked in its conversation (PDT-502): the table of its decisions, where the user unticks the
// rows they don't want, then allows the rest or denies the call. The same request is the Inbox's approval item.
import { ApprovalCard, type ApprovalAnswer } from '@ui';
import type { DecideRow } from '../api/gen/DecideRow';
import { DecideRows, useRowsDecision } from '../common/DecideRows';

export function DecideCard({ id, rows, keyboard, onAnswer }: {
  /** The card's DOM id. */
  id: string;
  rows: DecideRow[];
  keyboard: boolean;
  /** `kept`: the rows an Allow keeps. */
  onAnswer: (answer: ApprovalAnswer, kept?: number[]) => Promise<unknown>;
}) {
  const d = useRowsDecision(rows);
  return (
    <ApprovalCard
      id={id}
      title={d.title}
      preview={<DecideRows rows={rows} kept={d.isKept} onToggle={d.toggle} />}
      description={<>{d.help} <span aria-live="polite">{d.ticked}.</span></>}
      approveLabel={d.allowTitle}
      approveDisabled={!d.canAllow}
      keyboard={keyboard}
      onAnswer={(a) => onAnswer(a, a.type === 'approve' ? d.kept : undefined)}
    />
  );
}
