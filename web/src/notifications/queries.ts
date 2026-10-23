import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { toast } from '@ui';
import { api } from '../api/client';
import { actionError } from '../common/useAction';
import type { Show } from '../api/gen/Show';

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

/** Every kind with its Show (Settings → Notifications). */
export function useNotificationSettings() {
  return useQuery({ queryKey: ['notifications', 'settings'], queryFn: api.notificationSettings });
}

export function useSetNotificationSettings() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (changes: Record<string, Show | null>) => api.setNotificationSettings(changes),
    onSuccess: (kinds) => qc.setQueryData(['notifications', 'settings'], kinds),
  });
}
