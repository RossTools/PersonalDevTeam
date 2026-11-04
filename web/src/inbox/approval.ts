import type { ApprovalAnswer } from '@ui';
import type { InboxItem } from '../api/gen/InboxItem';
import { answerRequest } from '../common/answerRequest';

/** Answers an approval item's permission request in its session; `onAnswered` once it went through. `rows`: the rows an
 * Allow of the PM's decisions keeps (PDT-502). */
export async function answerApproval(item: InboxItem, a: ApprovalAnswer, onAnswered: () => void, rows?: number[]) {
  const requestId = typeof item.body.requestId === 'string' ? item.body.requestId : '';
  if (await answerRequest(item.sessionId!, requestId, a, rows)) onAnswered();
}
