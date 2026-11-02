import { request } from './core';
import type { StuckAgentsView } from './gen/StuckAgentsView';

/** "When an agent is stuck" (Settings → Agents): the built-in workflow's inputs. */
export const stuckAgentsApi = {
  stuckAgents: () => request<StuckAgentsView>('/api/stuck-agents'),
  saveStuckAgents: (values: Record<string, unknown>) =>
    request<StuckAgentsView>('/api/stuck-agents', { method: 'PUT', body: JSON.stringify({ values }) }),
  resetStuckAgents: () => request<StuckAgentsView>('/api/stuck-agents/reset', { method: 'POST' }),
};
