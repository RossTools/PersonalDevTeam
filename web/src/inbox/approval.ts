import { toast, type ApprovalAnswer } from '@ui';
import { api, ApiError } from '../api/client';
import type { InboxItem } from '../api/gen/InboxItem';
import { actionError } from '../common/useAction';

/** Answers an approval item's permission request in its session; `onAnswered` once it went through. `rows`: the rows an
 * Allow of the PM's decisions keeps (PDT-502). */
export async function answerApproval(item: InboxItem, a: ApprovalAnswer, onAnswered: () => void, rows?: number[]) {
  const requestId = typeof item.body.requestId === 'string' ? item.body.requestId : '';
  try {
    await api.answerRequest(item.sessionId!, requestId, a, rows);
    onAnswered();
  } catch (err) {
    if (err instanceof ApiError && err.code === 'request_closed') toast.info('That request was already answered or has expired.');
    else toast.error(actionError(err));
  }
}
