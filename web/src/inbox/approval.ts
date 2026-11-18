import type { ApprovalAnswer } from '@ui';
import type { InboxItem } from '../api/gen/InboxItem';
import { answerRequest } from '../common/answerRequest';
import { beginAnswer, endAnswer } from '../common/answerGuard';
import { answerSource } from '../common/answerSource';
import { holdAnswer, undoToast } from './held';

/** What an approval's answer says in its Undo toast. */
function label(a: ApprovalAnswer): string {
  switch (a.type) {
    case 'approve': return a.always ? 'Always allow' : 'Allow';
    case 'deny': return 'Deny';
    case 'question': return 'Answered';
    case 'plan': return a.approve ? 'Approve plan' : 'Keep planning';
  }
}

/**
 * Answers an approval item's permission request in its session, held for Undo like the Inbox's other answers (PDT-623);
 * `onAnswered` at once, `then` once it went out (refresh the Inbox), `onUndo` when it is undone. `rows`: the rows an
 * Allow of the PM's decisions keeps (PDT-502). Nothing happens right after another answer.
 */
export function answerApproval(item: InboxItem, a: ApprovalAnswer, onAnswered: () => void, opts: {
  rows?: number[];
  phone?: boolean;
  then?: () => Promise<unknown>;
  onUndo?: () => void;
} = {}) {
  if (!beginAnswer()) return;
  try {
    const requestId = typeof item.body.requestId === 'string' ? item.body.requestId : '';
    const source = answerSource(opts.phone);
    const said = label(a);
    holdAnswer({
      itemId: item.id, title: item.title, label: said,
      send: async () => {
        await answerRequest(item.sessionId!, requestId, a, opts.rows, source);
        await opts.then?.();
      },
    });
    undoToast(item.id, item.title, said, opts.onUndo);
    onAnswered();
  } finally {
    endAnswer();
  }
}
