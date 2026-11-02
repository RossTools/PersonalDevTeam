import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { toast } from '@ui';
import { api } from '../api/client';
import { actionError } from '../common/useAction';
import type { NotificationList } from '../api/gen/NotificationList';
import type { NotificationRulesView } from '../api/gen/NotificationRulesView';

/** The Inbox's Updates, newest first, polled with the rest of the UI; `unreadUpdates` is the rail's dot. */
export function useUpdates() {
  return useQuery({ queryKey: ['notifications', 'updates'], queryFn: () => api.notifications({ section: 'updates', limit: 100 }), refetchInterval: 2000 });
}

export function useRefreshNotifications() {
  const qc = useQueryClient();
  return () => Promise.all([
    qc.invalidateQueries({ queryKey: ['notifications'] }),
    qc.invalidateQueries({ queryKey: ['inbox'] }),
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

/** Clears one Update from the Inbox (PDT-224); it is gone from the list at once. */
export function useClearUpdate() {
  const qc = useQueryClient();
  const refresh = useRefreshNotifications();
  return useMutation({
    mutationFn: (id: string) => api.clearNotification(id),
    onMutate: (id) => qc.setQueryData<NotificationList>(['notifications', 'updates'], (l) => l && { ...l, items: l.items.filter((u) => u.id !== id) }),
    onError: (e) => toast.error(`Couldn't clear the update: ${actionError(e)}`),
    onSettled: () => void refresh(),
  });
}

/** Clears every Update ("Clear all"). */
export function useClearAllUpdates() {
  const qc = useQueryClient();
  const refresh = useRefreshNotifications();
  return useMutation({
    mutationFn: () => api.clearUpdates(),
    onMutate: () => qc.setQueryData<NotificationList>(['notifications', 'updates'], (l) => l && { ...l, items: [], unreadUpdates: 0 }),
    onError: (e) => toast.error(`Couldn't clear the updates: ${actionError(e)}`),
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
    void qc.invalidateQueries({ queryKey: ['workflow', v.workflowId] });
  };
  const save = useMutation({ meta: { errorShown: true }, mutationFn: api.saveNotificationRules, onSuccess: done });
  const reset = useMutation({ mutationFn: api.resetNotificationRules, onSuccess: done });
  const enable = useMutation({
    meta: { errorShown: true },
    mutationFn: ({ id, on }: { id: string; on: boolean }) => api.patchWorkflow(id, { enabled: on }),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: RULES });
      void qc.invalidateQueries({ queryKey: ['workflows'] });
    },
  });
  return { save, reset, enable };
}
