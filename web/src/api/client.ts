import type { ApiErrorBody } from './gen/ApiErrorBody';
import type { GuideInfo } from './gen/GuideInfo';
import type { Health } from './gen/Health';
import type { TicketDraft } from './gen/TicketDraft';
import type { HostChecks } from './gen/HostChecks';
import type { HostSettingsPatch } from './gen/HostSettingsPatch';
import type { HostSettingsView } from './gen/HostSettingsView';
import type { Branch } from './gen/Branch';
import type { NewProject } from './gen/NewProject';
import type { CloneProject } from './gen/CloneProject';
import type { FileContent } from './gen/FileContent';
import type { FileEntry } from './gen/FileEntry';
import type { FolderEntry } from './gen/FolderEntry';
import type { FolderListing } from './gen/FolderListing';
import type { NewFolder } from './gen/NewFolder';
import type { ProjectPatch } from './gen/ProjectPatch';
import type { NewRun } from './gen/NewRun';
import type { NewSession } from './gen/NewSession';
import type { NewWorktree } from './gen/NewWorktree';
import type { Project } from './gen/Project';
import type { PullBranch } from './gen/PullBranch';
import type { PullResult } from './gen/PullResult';
import type { ProjectSettingsInput } from './gen/ProjectSettingsInput';
import type { ProjectSettingsView } from './gen/ProjectSettingsView';
import type { RemoteStatus } from './gen/RemoteStatus';
import type { Run } from './gen/Run';
import type { RunConfigView } from './gen/RunConfigView';
import type { Attachment } from './gen/Attachment';
import type { Session } from './gen/Session';
import type { SessionMode } from './gen/SessionMode';
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
import type { NewParts } from './gen/NewParts';
import type { PartsCreated } from './gen/PartsCreated';
import type { RankMove } from './gen/RankMove';
import type { ProjectAreas } from './gen/ProjectAreas';
import type { Ticket } from './gen/Ticket';
import type { TicketEvent } from './gen/TicketEvent';
import type { TicketLink } from './gen/TicketLink';
import type { TicketPage } from './gen/TicketPage';
import type { TicketPatch } from './gen/TicketPatch';
import type { TicketQuery } from './gen/TicketQuery';
import type { StartResult } from './gen/StartResult';
import type { StartTicket } from './gen/StartTicket';
import type { AgentCapabilities } from './gen/AgentCapabilities';
import type { AgentKind } from './gen/AgentKind';
import type { Answer } from './gen/Answer';
import type { AnswerRequest } from './gen/AnswerRequest';
import type { ConversationEvent } from './gen/ConversationEvent';
import type { PendingMessage } from './gen/PendingMessage';
import type { PermissionMode } from './gen/PermissionMode';
import type { SendMessage } from './gen/SendMessage';
import type { Changes } from './gen/Changes';
import type { FeedbackResult } from './gen/FeedbackResult';
import type { FileSides } from './gen/FileSides';
import type { MergePreview } from './gen/MergePreview';
import type { MergeResult } from './gen/MergeResult';
import type { NewReviewComment } from './gen/NewReviewComment';
import type { ReviewComment } from './gen/ReviewComment';
import type { ReviewCommentPatch } from './gen/ReviewCommentPatch';

/** `PATCH /api/sessions/{id}`: any of these; `model: null` goes back to the agent's default. */
export interface SessionPatch {
  title?: string;
  model?: string | null;
  permissionMode?: PermissionMode;
}

export class ApiError extends Error {
  constructor(public status: number, public code: string, message: string) {
    super(message);
    this.name = 'ApiError';
  }
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await send(path, init);
  if (res.status === 204) return undefined as T;
  return (await res.json()) as T;
}

/** A plain-text response (PDT's guides are Markdown). */
async function requestText(path: string): Promise<string> {
  return (await send(path)).text();
}

async function send(path: string, init?: RequestInit): Promise<Response> {
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
  return res;
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
  listFolders: (q: { path?: string; hidden?: boolean; nearest?: boolean }) =>
    request<FolderListing>(`/api/host/folders${queryString({ ...q, hidden: q.hidden || undefined, nearest: q.nearest || undefined })}`),
  createFolder: (f: NewFolder) => request<FolderEntry>('/api/host/folders', { method: 'POST', body: JSON.stringify(f) }),
  listProjects: () => request<Project[]>('/api/projects'),
  createProject: (p: NewProject) => request<Project>('/api/projects', { method: 'POST', body: JSON.stringify(p) }),
  cloneProject: (p: CloneProject) => request<Project>('/api/projects/clone', { method: 'POST', body: JSON.stringify(p) }),
  updateProject: (id: string, p: ProjectPatch) => request<Project>(`/api/projects/${enc(id)}`, { method: 'PATCH', body: JSON.stringify(p) }),
  pullBranch: (id: string, p: PullBranch = {}) =>
    request<PullResult>(`/api/projects/${enc(id)}/pull`, { method: 'POST', body: JSON.stringify(p) }),
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
  getSession: (id: string) => request<Session>(`/api/sessions/${enc(id)}`),
  listSessions: (projectId: string) => request<Session[]>(`/api/projects/${enc(projectId)}/sessions`),
  createSession: (s: NewSession) => request<Session>('/api/sessions', { method: 'POST', body: JSON.stringify(s) }),
  viewSession: (id: string) => request<void>(`/api/sessions/${enc(id)}/view`, { method: 'POST' }),
  killSession: (id: string) => request<void>(`/api/sessions/${enc(id)}/kill`, { method: 'POST' }),
  restartSession: (id: string, size: TerminalSize) =>
    request<Session>(`/api/sessions/${enc(id)}/restart`, { method: 'POST', body: JSON.stringify(size) }),
  renameSession: (id: string, title: string) =>
    request<Session>(`/api/sessions/${enc(id)}`, { method: 'PATCH', body: JSON.stringify({ title }) }),
  deleteSession: (id: string) => request<void>(`/api/sessions/${enc(id)}`, { method: 'DELETE' }),
  updateSession: (id: string, p: SessionPatch) =>
    request<Session>(`/api/sessions/${enc(id)}`, { method: 'PATCH', body: JSON.stringify(p) }),
  /** Make ticket: the agent drafts a title and description from the conversation (a hidden side request). */
  draftTicket: (id: string, signal?: AbortSignal) =>
    request<TicketDraft>(`/api/sessions/${enc(id)}/ticket-draft`, { method: 'POST', signal }),
  /** A conversation's stored events after `after`, oldest first. */
  listSessionEvents: (id: string, q: { after?: number; limit?: number } = {}) =>
    request<ConversationEvent[]>(`/api/sessions/${enc(id)}/events${queryString(q)}`),
  /** Sends a message to a conversation (reopening it when parked or ended); it is pending until the agent takes it. */
  sendMessage: (id: string, m: SendMessage) =>
    request<PendingMessage>(`/api/sessions/${enc(id)}/messages`, { method: 'POST', body: JSON.stringify(m) }),
  /** Answers an open request; 409 `request_closed` when it was answered elsewhere or expired. */
  answerRequest: (id: string, requestId: string, answer: Answer) =>
    request<void>(`/api/sessions/${enc(id)}/requests/${enc(requestId)}`, {
      method: 'POST',
      body: JSON.stringify({ answer } satisfies AnswerRequest),
    }),
  interruptSession: (id: string) => request<void>(`/api/sessions/${enc(id)}/interrupt`, { method: 'POST' }),
  /** Codex conversations: opens the agent's own TUI on the same thread, live (PTY at the session's pty socket). */
  openSessionTerminal: (id: string, size: TerminalSize) =>
    request<void>(`/api/sessions/${enc(id)}/terminal`, { method: 'POST', body: JSON.stringify(size) }),
  closeSessionTerminal: (id: string) => request<void>(`/api/sessions/${enc(id)}/terminal`, { method: 'DELETE' }),
  /** Claude: hand the session between the conversation and its own TUI (`interrupt` stops a running turn). */
  switchSessionMode: (id: string, body: { mode: SessionMode; cols?: number; rows?: number; interrupt?: boolean }) =>
    request<Session>(`/api/sessions/${enc(id)}/mode`, { method: 'POST', body: JSON.stringify(body) }),
  agentCapabilities: (agent: AgentKind) => request<AgentCapabilities>(`/api/agents/${enc(agent)}/capabilities`),
  /** Stores an image for a message (PNG, JPEG, GIF or WebP, ≤ 10 MB); send its id in `images`. */
  uploadAttachment: (sessionId: string, file: Blob) =>
    request<Attachment>(`/api/sessions/${enc(sessionId)}/attachments`, {
      method: 'POST',
      body: file,
      headers: { 'content-type': file.type || 'application/octet-stream' },
    }),
  /** Where an attached image is served (thumbnails). */
  attachmentUrl: (sessionId: string, attachmentId: string) =>
    `/api/sessions/${enc(sessionId)}/attachments/${enc(attachmentId)}`,
  /** The session folder's files best matching an `@` query. */
  sessionFiles: (sessionId: string, q: string) =>
    request<string[]>(`/api/sessions/${enc(sessionId)}/files${queryString({ q })}`),
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
  /** Several parts of a parent, in order after its existing parts (PDT-38). */
  createParts: (id: string, b: NewParts) =>
    request<PartsCreated>(`/api/tickets/${enc(id)}/parts`, { method: 'POST', body: JSON.stringify(b) }),
  startTicket: (id: string, b: StartTicket) =>
    request<StartResult>(`/api/tickets/${enc(id)}/start`, { method: 'POST', body: JSON.stringify(b) }),
  checkMerge: (id: string) => request<Ticket>(`/api/tickets/${enc(id)}/check-merge`, { method: 'POST' }),
  abandonAttempt: (id: string) => request<Ticket>(`/api/attempts/${enc(id)}/abandon`, { method: 'POST' }),
  approveAttempt: (id: string) => request<Ticket>(`/api/attempts/${enc(id)}/approve`, { method: 'POST' }),
  mergePreview: (id: string) => request<MergePreview>(`/api/attempts/${enc(id)}/merge`),
  mergeAttempt: (id: string) => request<MergeResult>(`/api/attempts/${enc(id)}/merge`, { method: 'POST' }),
  sendConflicts: (id: string) => request<FeedbackResult>(`/api/attempts/${enc(id)}/conflicts`, { method: 'POST' }),
  getChanges: (worktreeId: string) => request<Changes>(`/api/worktrees/${enc(worktreeId)}/changes`),
  getFileSides: (worktreeId: string, path: string, oldPath?: string | null) =>
    request<FileSides>(`/api/worktrees/${enc(worktreeId)}/changes/file${queryString({ path, oldPath })}`),
  listReviewComments: (worktreeId: string) => request<ReviewComment[]>(`/api/worktrees/${enc(worktreeId)}/comments`),
  createReviewComment: (worktreeId: string, c: NewReviewComment) =>
    request<ReviewComment>(`/api/worktrees/${enc(worktreeId)}/comments`, { method: 'POST', body: JSON.stringify(c) }),
  updateReviewComment: (id: string, p: ReviewCommentPatch) =>
    request<ReviewComment>(`/api/review-comments/${enc(id)}`, { method: 'PATCH', body: JSON.stringify(p) }),
  deleteReviewComment: (id: string) => request<void>(`/api/review-comments/${enc(id)}`, { method: 'DELETE' }),
  sendFeedback: (worktreeId: string) => request<FeedbackResult>(`/api/worktrees/${enc(worktreeId)}/feedback`, { method: 'POST' }),
  projectLabels: (projectId: string) => request<string[]>(`/api/projects/${enc(projectId)}/labels`),
  /** The areas `.pdt/project.toml` (and the host overrides) declare, or why there are none. */
  getProjectAreas: (projectId: string) => request<ProjectAreas>(`/api/projects/${enc(projectId)}/areas`),
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
  listGuides: () => request<GuideInfo[]>('/api/guides'),
  /** A built-in guide or prompt, as Markdown. */
  guide: (name: string) => requestText(`/api/guides/${enc(name)}`),
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
