import { useMutation, useQuery, useQueryClient, type QueryClient } from '@tanstack/react-query';
import { toast } from '@ui';
import { api } from '../api/client';
import { actionError } from '../common/useAction';
import { invalidateWorkflow, workflowKeys } from '../workflows/queries';
import type { NotificationList } from '../api/gen/NotificationList';
import type { NotificationRulesView } from '../api/gen/NotificationRulesView';
import { streamPoll } from '../common/poll';
import { inboxKey } from '../inbox/useInbox';

const UPDATES = ['notifications', 'updates'];

/**
 * The Inbox's Updates, newest first; `unreadUpdates` is the rail's dot. The notification stream pushes them, so they
 * poll slowly while it is connected (every 2 s while it isn't).
 */
export function useUpdates() {
  return useQuery({ queryKey: UPDATES, queryFn: () => api.notifications({ section: 'updates', limit: 100 }), refetchInterval: streamPoll });
}

function useRefreshNotifications() {
  const qc = useQueryClient();
  return () => Promise.all([
    qc.invalidateQueries({ queryKey: ['notifications'] }),
    qc.invalidateQueries({ queryKey: inboxKey }),
  ]);
}

export function useReadNotification() {
  const refresh = useRefreshNotifications();
  return useMutation({
    mutationFn: (id: string) => api.readNotification(id),
    onError: (e) => toast.error(`Couldn't mark the update read: ${actionError(e)}`),
    onSettled: () => void refresh(),
  });
}

export function useReadAllUpdates() {
  const refresh = useRefreshNotifications();
  return useMutation({
    mutationFn: () => api.readNotifications({ section: 'updates' }),
    onError: (e) => toast.error(`Couldn't mark the updates read: ${actionError(e)}`),
    onSettled: () => void refresh(),
  });
}

/**
 * Shows the Updates as `change` leaves them before the server answers, and gives back the list as it was (for a
 * failure). A poll in flight would land the old list over it (PDT-451), so it is cancelled first.
 */
async function optimistic(qc: QueryClient, change: (l: NotificationList) => NotificationList) {
  await qc.cancelQueries({ queryKey: UPDATES });
  const previous = qc.getQueryData<NotificationList>(UPDATES);
  if (previous) qc.setQueryData(UPDATES, change(previous));
  return { previous };
}

const rollBack = (qc: QueryClient, context: { previous?: NotificationList } | undefined) => {
  if (context?.previous) qc.setQueryData(UPDATES, context.previous);
};

/** Clears one Update from the Inbox (PDT-224); it is gone from the list at once (back if clearing fails). */
export function useClearUpdate() {
  const qc = useQueryClient();
  const refresh = useRefreshNotifications();
  return useMutation({
    mutationFn: (id: string) => api.clearNotification(id),
    onMutate: (id) => optimistic(qc, (l) => ({ ...l, items: l.items.filter((u) => u.id !== id) })),
    onError: (e, _id, context) => {
      rollBack(qc, context);
      toast.error(`Couldn't clear the update: ${actionError(e)}`);
    },
    onSettled: () => void refresh(),
  });
}

/** Clears every Update ("Clear all"). */
export function useClearAllUpdates() {
  const qc = useQueryClient();
  const refresh = useRefreshNotifications();
  return useMutation({
    mutationFn: () => api.clearUpdates(),
    onMutate: () => optimistic(qc, (l) => ({ ...l, items: [], unreadUpdates: 0 })),
    onError: (e, _v, context) => {
      rollBack(qc, context);
      toast.error(`Couldn't clear the updates: ${actionError(e)}`);
    },
    onSettled: () => void refresh(),
  });
}

const RULES = ['notifications', 'rules'];

/** The Notification rules: every kind's level and the other rules (Settings → Notifications). */
export function useNotificationRules() {
  return useQuery({ queryKey: RULES, queryFn: api.notificationRules });
}

/** Changes, resets or turns the Notification rules on or off; each answers with the rules as they are now. */
export function useNotificationRulesActions() {
  const qc = useQueryClient();
  const done = (v: NotificationRulesView) => {
    qc.setQueryData(RULES, v);
    void qc.invalidateQueries({ queryKey: workflowKeys.one(v.workflowId) });
  };
  const save = useMutation({ meta: { errorShown: true }, mutationFn: api.saveNotificationRules, onSuccess: done });
  const reset = useMutation({ mutationFn: api.resetNotificationRules, onSuccess: done });
  const enable = useMutation({
    meta: { errorShown: true },
    mutationFn: ({ id, on }: { id: string; on: boolean }) => api.patchWorkflow(id, { enabled: on }),
    onSuccess: (_d, { id }) => invalidateWorkflow(qc, id),
  });
  return { save, reset, enable };
}
