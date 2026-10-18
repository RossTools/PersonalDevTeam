import type { ApiErrorBody } from './gen/ApiErrorBody';
import type { Health } from './gen/Health';
import type { HostChecks } from './gen/HostChecks';
import type { HostSettingsPatch } from './gen/HostSettingsPatch';
import type { HostSettingsView } from './gen/HostSettingsView';
import type { Branch } from './gen/Branch';
import type { NewProject } from './gen/NewProject';
import type { CloneProject } from './gen/CloneProject';
import type { FileContent } from './gen/FileContent';
import type { FileEntry } from './gen/FileEntry';
import type { ProjectPatch } from './gen/ProjectPatch';
import type { NewRun } from './gen/NewRun';
import type { NewSession } from './gen/NewSession';
import type { NewWorktree } from './gen/NewWorktree';
import type { Project } from './gen/Project';
import type { ProjectSettingsInput } from './gen/ProjectSettingsInput';
import type { ProjectSettingsView } from './gen/ProjectSettingsView';
import type { RemoteStatus } from './gen/RemoteStatus';
import type { Run } from './gen/Run';
import type { RunConfigView } from './gen/RunConfigView';
import type { Session } from './gen/Session';
import type { TerminalSize } from './gen/TerminalSize';
import type { UpdateStatus } from './gen/UpdateStatus';
import type { WhoAmI } from './gen/WhoAmI';
import type { Worktree } from './gen/Worktree';
import type { NewWorkspace } from './gen/NewWorkspace';
import type { OpenTab } from './gen/OpenTab';
import type { Tab } from './gen/Tab';
import type { TabPatch } from './gen/TabPatch';
import type { Workspace } from './gen/Workspace';
import type { WorkspacePatch } from './gen/WorkspacePatch';
import type { BulkRequest } from './gen/BulkRequest';
import type { BulkResult } from './gen/BulkResult';
import type { EventPage } from './gen/EventPage';
import type { EventQuery } from './gen/EventQuery';
import type { NewComment } from './gen/NewComment';
import type { NewTicket } from './gen/NewTicket';
import type { RankMove } from './gen/RankMove';
import type { Ticket } from './gen/Ticket';
import type { TicketEvent } from './gen/TicketEvent';
import type { TicketLink } from './gen/TicketLink';
import type { TicketPage } from './gen/TicketPage';
import type { TicketPatch } from './gen/TicketPatch';
import type { TicketQuery } from './gen/TicketQuery';
import type { StartResult } from './gen/StartResult';
import type { StartTicket } from './gen/StartTicket';

export class ApiError extends Error {
  constructor(public status: number, public code: string, message: string) {
    super(message);
    this.name = 'ApiError';
  }
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await fetch(path, { ...init, headers: { 'content-type': 'application/json', ...init?.headers } });
  if (!res.ok) {
    let body: Partial<ApiErrorBody> = {};
    try {
      body = await res.json();
    } catch {
      // non-JSON error body (proxy, crash page)
    }
    throw new ApiError(res.status, body.code ?? 'http_error', body.message ?? `HTTP ${res.status}`);
  }
  if (res.status === 204) return undefined as T;
  return (await res.json()) as T;
}

const enc = encodeURIComponent;

/** `?a=1&b=x` from the defined, non-empty fields (empty string when there are none). */
function queryString(q: object): string {
  const params = new URLSearchParams();
  for (const [k, v] of Object.entries(q)) {
    if (v !== undefined && v !== null && v !== '') params.set(k, String(v));
  }
  const s = params.toString();
  return s ? `?${s}` : '';
}

function filesQuery(worktreeId: string | null, path: string): string {
  return new URLSearchParams({ ...(worktreeId ? { worktreeId } : {}), path }).toString();
}

export const api = {
  health: () => request<Health>('/api/health'),
  listProjects: () => request<Project[]>('/api/projects'),
  createProject: (p: NewProject) => request<Project>('/api/projects', { method: 'POST', body: JSON.stringify(p) }),
  cloneProject: (p: CloneProject) => request<Project>('/api/projects/clone', { method: 'POST', body: JSON.stringify(p) }),
  updateProject: (id: string, p: ProjectPatch) => request<Project>(`/api/projects/${enc(id)}`, { method: 'PATCH', body: JSON.stringify(p) }),
  deleteProject: (id: string) => request<void>(`/api/projects/${enc(id)}`, { method: 'DELETE' }),
  removeWorktree: (id: string, opts: { force?: boolean; deleteBranch?: boolean } = {}) =>
    request<void>(`/api/worktrees/${enc(id)}?force=${opts.force ? 'true' : 'false'}&deleteBranch=${opts.deleteBranch ? 'true' : 'false'}`, { method: 'DELETE' }),
  listFiles: (projectId: string, worktreeId: string | null, path: string) =>
    request<FileEntry[]>(`/api/projects/${enc(projectId)}/files?${filesQuery(worktreeId, path)}`),
  fileContent: (projectId: string, worktreeId: string | null, path: string) =>
    request<FileContent>(`/api/projects/${enc(projectId)}/files/content?${filesQuery(worktreeId, path)}`),
  listWorktrees: (projectId: string) => request<Worktree[]>(`/api/projects/${enc(projectId)}/worktrees`),
  createWorktree: (projectId: string, w: NewWorktree) =>
    request<Worktree>(`/api/projects/${enc(projectId)}/worktrees`, { method: 'POST', body: JSON.stringify(w) }),
  listSessions: (projectId: string) => request<Session[]>(`/api/projects/${enc(projectId)}/sessions`),
  createSession: (s: NewSession) => request<Session>('/api/sessions', { method: 'POST', body: JSON.stringify(s) }),
  viewSession: (id: string) => request<void>(`/api/sessions/${enc(id)}/view`, { method: 'POST' }),
  killSession: (id: string) => request<void>(`/api/sessions/${enc(id)}/kill`, { method: 'POST' }),
  restartSession: (id: string, size: TerminalSize) =>
    request<Session>(`/api/sessions/${enc(id)}/restart`, { method: 'POST', body: JSON.stringify(size) }),
  renameSession: (id: string, title: string) =>
    request<Session>(`/api/sessions/${enc(id)}`, { method: 'PATCH', body: JSON.stringify({ title }) }),
  deleteSession: (id: string) => request<void>(`/api/sessions/${enc(id)}`, { method: 'DELETE' }),
  listWorkspaces: () => request<Workspace[]>('/api/workspaces'),
  createWorkspace: (w: NewWorkspace) => request<Workspace>('/api/workspaces', { method: 'POST', body: JSON.stringify(w) }),
  updateWorkspace: (id: string, p: WorkspacePatch) =>
    request<Workspace>(`/api/workspaces/${enc(id)}`, { method: 'PATCH', body: JSON.stringify(p) }),
  deleteWorkspace: (id: string) => request<void>(`/api/workspaces/${enc(id)}`, { method: 'DELETE' }),
  openTab: (workspaceId: string, t: OpenTab) =>
    request<Tab>(`/api/workspaces/${enc(workspaceId)}/tabs`, { method: 'POST', body: JSON.stringify(t) }),
  updateTab: (id: string, p: TabPatch) => request<Tab>(`/api/tabs/${enc(id)}`, { method: 'PATCH', body: JSON.stringify(p) }),
  closeTab: (id: string) => request<void>(`/api/tabs/${enc(id)}`, { method: 'DELETE' }),
  listTickets: (q: TicketQuery = {}) => request<TicketPage>(`/api/tickets${queryString(q)}`),
  createTicket: (projectId: string, t: NewTicket) =>
    request<Ticket>(`/api/projects/${enc(projectId)}/tickets`, { method: 'POST', body: JSON.stringify(t) }),
  getTicket: (id: string) => request<Ticket>(`/api/tickets/${enc(id)}`),
  updateTicket: (id: string, p: TicketPatch) =>
    request<Ticket>(`/api/tickets/${enc(id)}`, { method: 'PATCH', body: JSON.stringify(p) }),
  deleteTicket: (id: string) => request<void>(`/api/tickets/${enc(id)}`, { method: 'DELETE' }),
  rankTicket: (id: string, m: RankMove) =>
    request<Ticket>(`/api/tickets/${enc(id)}/rank`, { method: 'POST', body: JSON.stringify(m) }),
  bulkTickets: (b: BulkRequest) => request<BulkResult>('/api/tickets/bulk', { method: 'POST', body: JSON.stringify(b) }),
  linkTicket: (id: string, l: TicketLink) =>
    request<Ticket>(`/api/tickets/${enc(id)}/links`, { method: 'POST', body: JSON.stringify(l) }),
  unlinkTicket: (id: string, l: TicketLink) =>
    request<Ticket>(`/api/tickets/${enc(id)}/links`, { method: 'DELETE', body: JSON.stringify(l) }),
  ticketEvents: (id: string, q: EventQuery = {}) =>
    request<EventPage>(`/api/tickets/${enc(id)}/events${queryString(q)}`),
  commentOnTicket: (id: string, body: string) =>
    request<TicketEvent>(`/api/tickets/${enc(id)}/comments`, { method: 'POST', body: JSON.stringify({ body } satisfies NewComment) }),
  startTicket: (id: string, b: StartTicket) =>
    request<StartResult>(`/api/tickets/${enc(id)}/start`, { method: 'POST', body: JSON.stringify(b) }),
  checkMerge: (id: string) => request<Ticket>(`/api/tickets/${enc(id)}/check-merge`, { method: 'POST' }),
  abandonAttempt: (id: string) => request<Ticket>(`/api/attempts/${enc(id)}/abandon`, { method: 'POST' }),
  projectLabels: (projectId: string) => request<string[]>(`/api/projects/${enc(projectId)}/labels`),
  listRuns: (projectId: string) => request<Run[]>(`/api/projects/${enc(projectId)}/runs`),
  listActiveRuns: () => request<Run[]>('/api/runs'),
  startRun: (projectId: string, r: NewRun) =>
    request<Run>(`/api/projects/${enc(projectId)}/runs`, { method: 'POST', body: JSON.stringify(r) }),
  stopRun: (id: string) => request<void>(`/api/runs/${enc(id)}/stop`, { method: 'POST' }),
  restartRun: (id: string, stopRunId?: string) =>
    request<Run>(`/api/runs/${enc(id)}/restart`, { method: 'POST', body: JSON.stringify(stopRunId ? { stopRunId } : {}) }),
  resetRunData: (id: string) => request<void>(`/api/runs/${enc(id)}/reset-data`, { method: 'POST' }),
  deleteRun: (id: string) => request<void>(`/api/runs/${enc(id)}`, { method: 'DELETE' }),
  runConfig: (projectId: string, worktreeId: string | null) =>
    request<RunConfigView>(
      `/api/projects/${enc(projectId)}/run-config${worktreeId ? `?worktreeId=${enc(worktreeId)}` : ''}`,
    ),
  listBranches: (projectId: string) => request<Branch[]>(`/api/projects/${enc(projectId)}/branches`),
  projectSettings: (projectId: string) => request<ProjectSettingsView>(`/api/projects/${enc(projectId)}/settings`),
  saveProjectSettings: (projectId: string, s: ProjectSettingsInput) =>
    request<ProjectSettingsView>(`/api/projects/${enc(projectId)}/settings`, { method: 'PUT', body: JSON.stringify(s) }),
  hostSettings: () => request<HostSettingsView>('/api/host/settings'),
  saveHostSettings: (p: HostSettingsPatch) =>
    request<HostSettingsView>('/api/host/settings', { method: 'PUT', body: JSON.stringify(p) }),
  refreshRemote: () => request<RemoteStatus>('/api/host/remote/refresh', { method: 'POST' }),
  whoami: () => request<WhoAmI>('/api/host/whoami'),
  updateStatus: () => request<UpdateStatus>('/api/host/update'),
  checkUpdate: () => request<UpdateStatus>('/api/host/update/check', { method: 'POST' }),
  installUpdate: (force: boolean) =>
    request<UpdateStatus>('/api/host/update/install', { method: 'POST', body: JSON.stringify({ force }) }),
  dismissUpdateResult: () => request<void>('/api/host/update/dismiss', { method: 'POST' }),
  saveToken: (token: string) => request<UpdateStatus>('/api/host/token', { method: 'PUT', body: JSON.stringify({ token }) }),
  deleteToken: () => request<void>('/api/host/token', { method: 'DELETE' }),
  hostChecks: () => request<HostChecks>('/api/host/checks'),
  refreshChecks: () => request<HostChecks>('/api/host/checks/refresh', { method: 'POST' }),
};
