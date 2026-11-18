import { enc, queryString, request } from './core';
import type { AnswerSource } from './gen/AnswerSource';
import type { InboxAnswer } from './gen/InboxAnswer';
import type { InboxList } from './gen/InboxList';
import type { NotificationList } from './gen/NotificationList';
import type { NotificationSection } from './gen/NotificationSection';
import type { ReadAll } from './gen/ReadAll';

/** How the user gave an answer, in the header pdtd reads it from (`ANSWER_SOURCE_HEADER`, PDT-623). */
export const sourceHeader = (s?: AnswerSource): Record<string, string> => (s ? { 'x-pdt-answer-source': `${s.via}/${s.device}` } : {});

/** The inbox (what waits on the user) and notifications. */
export const inboxApi = {
  /** What waits on the user: questions, approvals, stalled steps, loop limits, gates. */
  inbox: (project?: string) => request<InboxList>(`/api/inbox${queryString({ project })}`),
  /** `source`: how the user gave it (PDT-623). */
  answerInbox: (id: string, a: InboxAnswer, source?: AnswerSource) =>
    // As the user's: a triaged ticket's Approve lets an external ticket in (PDT-225).
    request<Record<string, unknown>>(`/api/inbox/${enc(id)}/answer`, { method: 'POST', body: JSON.stringify(a), headers: sourceHeader(source) }, { asUser: true }),
  dismissInbox: (id: string) => request<void>(`/api/inbox/${enc(id)}/dismiss`, { method: 'POST' }),
  notifications: (q: { section?: NotificationSection; project?: string; unread?: boolean; limit?: number; before?: number } = {}) =>
    request<NotificationList>(`/api/notifications${queryString(q)}`),
  readNotification: (id: string) => request<void>(`/api/notifications/${enc(id)}/read`, { method: 'POST' }),
  readNotifications: (r: ReadAll) => request<void>('/api/notifications/read-all', { method: 'POST', body: JSON.stringify(r) }),
  clearNotification: (id: string) => request<void>(`/api/notifications/${enc(id)}/clear`, { method: 'POST' }),
  clearUpdates: () => request<void>('/api/notifications/clear-all', { method: 'POST' }),
};
