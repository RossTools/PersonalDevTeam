import { useState } from 'react';
import { toast } from '@ui';
import { api, ApiError } from '../api/client';
import type { InboxAnswer } from '../api/gen/InboxAnswer';
import type { InboxItem } from '../api/gen/InboxItem';
import { actionError } from '../common/useAction';
import { dismissable } from './model';
import { useRefreshInbox } from './useInbox';

/**
 * Answering and dismissing the item on screen. `onDone` runs once the item is answered (or dismissed), before the
 * Inbox is fetched again: show the next one. An item someone already answered (409) says so and stays.
 */
export function useAnswer(item: InboxItem | null, onDone: (item: InboxItem) => void) {
  const refresh = useRefreshInbox();
  const [busy, setBusy] = useState(false);
  const answer = async (a: InboxAnswer) => {
    if (!item) return;
    setBusy(true);
    try {
      await api.answerInbox(item.id, a);
      onDone(item);
      await refresh();
    } catch (err) {
      if (err instanceof ApiError && err.status === 409) toast.info(err.message);
      else toast.error(actionError(err));
      await refresh();
    } finally {
      setBusy(false);
    }
  };
  const dismiss = async () => {
    if (!item || !dismissable(item)) return;
    try {
      await api.dismissInbox(item.id);
      onDone(item);
      await refresh();
    } catch (err) {
      toast.error(actionError(err));
    }
  };
  return { busy, answer, dismiss, refresh };
}
