import { request } from './core';
import type { NotificationRulesInput } from './gen/NotificationRulesInput';
import type { NotificationRulesView } from './gen/NotificationRulesView';

/** The Notification rules (Settings → Notifications): the built-in workflow's levels and other inputs. */
export const notificationRulesApi = {
  notificationRules: () => request<NotificationRulesView>('/api/notification-rules'),
  saveNotificationRules: (input: NotificationRulesInput) =>
    request<NotificationRulesView>('/api/notification-rules', { method: 'PUT', body: JSON.stringify(input) }),
  resetNotificationRules: () => request<NotificationRulesView>('/api/notification-rules/reset', { method: 'POST' }),
};
