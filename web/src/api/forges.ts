import { enc, request } from './core';
import type { FiledIssue } from './gen/FiledIssue';
import type { ForgeLoginView } from './gen/ForgeLoginView';
import type { ForgeTokenCheck } from './gen/ForgeTokenCheck';
import type { NewForgeIssue } from './gen/NewForgeIssue';
import type { ProjectForgeInfo } from './gen/ProjectForgeInfo';

/** Forge accounts (GitHub and the like) and a project's forge. */
export const forgesApi = {
  /** The project's forge (null: none), and whether issues can be filed there (PDT-221). */
  projectForge: (projectId: string) => request<ProjectForgeInfo | null>(`/api/projects/${enc(projectId)}/forge`),
  /** Files an issue in the project's repository as the user's login; with `ticketId`, that ticket links it. */
  createIssue: (projectId: string, issue: NewForgeIssue) =>
    request<FiledIssue>(`/api/projects/${enc(projectId)}/issues`, { method: 'POST', body: JSON.stringify(issue) }, { asUser: true }),
  /** A forge's accounts, one per host (PDT-244), and how to add one. */
  forgeLogin: (kind: string) => request<ForgeLoginView>(`/api/forges/${kind}/login`),
  /** Who a pasted token signs in as, without saving it. */
  forgeTestToken: (kind: string, token: string, apiUrl?: string) =>
    request<ForgeTokenCheck>(`/api/forges/${kind}/login/test`, { method: 'POST', body: JSON.stringify({ token, ...(apiUrl ? { apiUrl } : {}) }) }),
  forgeLoginToken: (kind: string, token: string, apiUrl?: string) =>
    request<ForgeLoginView>(`/api/forges/${kind}/login/token`, { method: 'POST', body: JSON.stringify({ token, ...(apiUrl ? { apiUrl } : {}) }) }),
  forgeLoginDevice: (kind: string, b: { apiUrl?: string; clientId?: string }) =>
    request<ForgeLoginView>(`/api/forges/${kind}/login/device`, { method: 'POST', body: JSON.stringify(b) }),
  forgeCancelDevice: (kind: string) => request<ForgeLoginView>(`/api/forges/${kind}/login/device`, { method: 'DELETE' }),
  forgeDisconnect: (kind: string, host: string) =>
    request<ForgeLoginView>(`/api/forges/${kind}/login/accounts/${encodeURIComponent(host)}`, { method: 'DELETE' }),
};
