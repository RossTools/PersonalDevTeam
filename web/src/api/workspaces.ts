import { enc, request } from './core';
import type { NewWorkspace } from './gen/NewWorkspace';
import type { OpenTab } from './gen/OpenTab';
import type { Tab } from './gen/Tab';
import type { TabPatch } from './gen/TabPatch';
import type { Workspace } from './gen/Workspace';
import type { WorkspacePatch } from './gen/WorkspacePatch';

/** Workspaces and their tabs. */
export const workspacesApi = {
  listWorkspaces: () => request<Workspace[]>('/api/workspaces'),
  createWorkspace: (w: NewWorkspace) => request<Workspace>('/api/workspaces', { method: 'POST', body: JSON.stringify(w) }),
  updateWorkspace: (id: string, p: WorkspacePatch) =>
    request<Workspace>(`/api/workspaces/${enc(id)}`, { method: 'PATCH', body: JSON.stringify(p) }),
  deleteWorkspace: (id: string, opts: { closeTabs?: boolean } = {}) =>
    request<void>(`/api/workspaces/${enc(id)}${opts.closeTabs ? '?closeTabs=true' : ''}`, { method: 'DELETE' }),
  openTab: (workspaceId: string, t: OpenTab) =>
    request<Tab>(`/api/workspaces/${enc(workspaceId)}/tabs`, { method: 'POST', body: JSON.stringify(t) }),
  updateTab: (id: string, p: TabPatch) => request<Tab>(`/api/tabs/${enc(id)}`, { method: 'PATCH', body: JSON.stringify(p) }),
  closeTab: (id: string) => request<void>(`/api/tabs/${enc(id)}`, { method: 'DELETE' }),
};
