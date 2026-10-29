import type { Notification } from '../api/gen/Notification';

export const notification = (id: string, over: Partial<Notification> = {}): Notification => ({
  id, seq: 1, kind: 'pr.merged', needsYou: false, severity: 'info', show: 'inbox', projectId: 'p', ticketId: 't',
  sessionId: null, runId: null, inboxItemId: null, title: `Update ${id}`, body: '', target: { kind: 'none' },
  dedupeKey: `k:${id}`, createdAt: Date.now(), readAt: null, clearedAt: null, dismissedAt: null, ...over,
});
