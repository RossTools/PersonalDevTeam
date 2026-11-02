import { request } from './core';
import type { InstallUpdatesView } from './gen/InstallUpdatesView';

/** Install updates (Settings → Updates): the built-in workflow's inputs. */
export const installUpdatesApi = {
  installUpdates: () => request<InstallUpdatesView>('/api/install-updates'),
  saveInstallUpdates: (values: Record<string, unknown>) =>
    request<InstallUpdatesView>('/api/install-updates', { method: 'PUT', body: JSON.stringify({ values }) }),
  resetInstallUpdates: () => request<InstallUpdatesView>('/api/install-updates/reset', { method: 'POST' }),
};
