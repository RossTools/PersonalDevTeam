import { toast, type ApprovalAnswer } from '@ui';
import { api, ApiError } from '../api/client';
import { actionError } from './useAction';

/**
 * Answers a session's open request (an approval, a question, a plan), from its conversation or the Inbox; `rows`: the
 * rows an Allow of the PM's decisions keeps (PDT-502). Toasts what went wrong; true once it went through.
 */
export async function answerRequest(sessionId: string, requestId: string, a: ApprovalAnswer, rows?: number[]): Promise<boolean> {
  try {
    await api.answerRequest(sessionId, requestId, a, rows);
    return true;
  } catch (err) {
    if (err instanceof ApiError && err.code === 'request_closed') toast.info('That request was already answered or has expired.');
    else toast.error(actionError(err));
    return false;
  }
}
