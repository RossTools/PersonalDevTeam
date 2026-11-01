import { enc, queryString, request } from './core';
import type { InboxAnswer } from './gen/InboxAnswer';
import type { InboxList } from './gen/InboxList';
import type { KindSetting } from './gen/KindSetting';
import type { NotificationList } from './gen/NotificationList';
import type { NotificationSection } from './gen/NotificationSection';
import type { ReadAll } from './gen/ReadAll';
import type { Show } from './gen/Show';

/** The inbox (what waits on the user) and notifications. */
export const inboxApi = {
  /** What waits on the user: questions, approvals, stalled steps, loop limits, gates (Phase 7b). */
  inbox: (project?: string) => request<InboxList>(`/api/inbox${queryString({ project })}`),
  answerInbox: (id: string, a: InboxAnswer) =>
    // As the user's: a triaged ticket's Approve lets an external ticket in (PDT-225).
    request<Record<string, unknown>>(`/api/inbox/${enc(id)}/answer`, { method: 'POST', body: JSON.stringify(a) }, { asUser: true }),
  dismissInbox: (id: string) => request<void>(`/api/inbox/${enc(id)}/dismiss`, { method: 'POST' }),
  notifications: (q: { section?: NotificationSection; project?: string; unread?: boolean; limit?: number; before?: number } = {}) =>
    request<NotificationList>(`/api/notifications${queryString(q)}`),
  readNotification: (id: string) => request<void>(`/api/notifications/${enc(id)}/read`, { method: 'POST' }),
  readNotifications: (r: ReadAll) => request<void>('/api/notifications/read-all', { method: 'POST', body: JSON.stringify(r) }),
  clearNotification: (id: string) => request<void>(`/api/notifications/${enc(id)}/clear`, { method: 'POST' }),
  clearUpdates: () => request<void>('/api/notifications/clear-all', { method: 'POST' }),
  notificationSettings: () => request<KindSetting[]>('/api/notifications/settings'),
  setNotificationSettings: (changes: Record<string, Show | null>) =>
    request<KindSetting[]>('/api/notifications/settings', { method: 'PATCH', body: JSON.stringify(changes) }),
};
