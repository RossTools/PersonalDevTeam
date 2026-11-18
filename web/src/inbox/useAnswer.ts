import { useState } from 'react';
import { toast } from '@ui';
import { api, ApiError } from '../api/client';
import type { InboxAnswer } from '../api/gen/InboxAnswer';
import type { InboxItem } from '../api/gen/InboxItem';
import { beginAnswer, endAnswer } from '../common/answerGuard';
import { answerSource } from '../common/answerSource';
import { actionError } from '../common/useAction';
import { holdAnswer, undoToast } from './held';
import { answerLabel, dismissable } from './model';
import { useRefreshInbox } from './useInbox';

/**
 * Answering and dismissing the item on screen. `onDone` runs once the item is answered (or dismissed): show the next
 * one. An answer is held for Undo (PDT-623) and goes out once its window passes; `onUndo` runs when it is undone (show
 * the item again). One that fails then, or that someone already answered (409), says so and the item comes back. A
 * second answer right after one (a double-click, a click on the item that slid into place) does nothing.
 */
export function useAnswer(item: InboxItem | null, onDone: (item: InboxItem) => void, opts: { phone?: boolean; onUndo?: (item: InboxItem) => void } = {}) {
  const refresh = useRefreshInbox();
  const [busy, setBusy] = useState(false);
  const answer = async (a: InboxAnswer) => {
    if (!item || !beginAnswer()) return;
    try {
      const it = item;
      const source = answerSource(opts.phone);
      const label = answerLabel(it, a);
      holdAnswer({
        itemId: it.id, title: it.title, label,
        send: async () => {
          try {
            await api.answerInbox(it.id, a, source);
          } catch (err) {
            if (err instanceof ApiError && err.status === 409) toast.info(err.message);
            else toast.error(actionError(err));
          }
          await refresh();
        },
      });
      undoToast(it.id, it.title, label, () => opts.onUndo?.(it));
      onDone(it);
    } finally {
      endAnswer();
    }
  };
  const dismiss = async () => {
    if (!item || !dismissable(item)) return;
    setBusy(true);
    try {
      await api.dismissInbox(item.id);
      onDone(item);
      await refresh();
    } catch (err) {
      toast.error(actionError(err));
    } finally {
      setBusy(false);
    }
  };
  return { busy, answer, dismiss, refresh };
}
