import type { ApiErrorBody } from './gen/ApiErrorBody';
import type { ConnectionKind } from './gen/ConnectionKind';
import type { BugReportFiled } from './gen/BugReportFiled';
import type { BugReportPreview } from './gen/BugReportPreview';
import type { NewBugReport } from './gen/NewBugReport';
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
import type { RenameFile } from './gen/RenameFile';
import type { SaveFile } from './gen/SaveFile';
import type { TellFilesChanged } from './gen/TellFilesChanged';
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
import type { JiraImportResult } from './gen/JiraImportResult';
import type { ProjectSettingsView } from './gen/ProjectSettingsView';
import type { RemoteStatus } from './gen/RemoteStatus';
import type { Run } from './gen/Run';
import type { RunConfigView } from './gen/RunConfigView';
import type { Attachment } from './gen/Attachment';
import type { Session } from './gen/Session';
import type { NewTodo } from './gen/NewTodo';
import type { Todo } from './gen/Todo';
import type { TodoPatch } from './gen/TodoPatch';
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
import type { MergeRequest } from './gen/MergeRequest';
import type { MergeResult } from './gen/MergeResult';
import type { NewReviewComment } from './gen/NewReviewComment';
import type { ReviewComment } from './gen/ReviewComment';
import type { ReviewCommentPatch } from './gen/ReviewCommentPatch';
import type { FinishRequest } from './gen/FinishRequest';
import type { FinishResult } from './gen/FinishResult';
import type { Workflow } from './gen/Workflow';
import type { DocDiff } from './gen/DocDiff';
import type { DryRunRequest } from './gen/DryRunRequest';
import type { DryRunResult } from './gen/DryRunResult';
import type { Instantiate } from './gen/Instantiate';
import type { KindSpec } from './gen/KindSpec';
import type { LogEvent } from './gen/LogEvent';
import type { Connection } from './gen/Connection';
import type { NewSecret } from './gen/NewSecret';
import type { Secret } from './gen/Secret';
import type { SecretPatch } from './gen/SecretPatch';
import type { SecretUse } from './gen/SecretUse';
import type { ConnectionInput } from './gen/ConnectionInput';
import type { McpTools } from './gen/McpTools';
import type { NewWorkflow } from './gen/NewWorkflow';
import type { SaveWorkflow } from './gen/SaveWorkflow';
import type { Started } from './gen/Started';
import type { TemplateInput } from './gen/TemplateInput';
import type { WorkflowCheck } from './gen/WorkflowCheck';
import type { WorkflowDetail } from './gen/WorkflowDetail';
import type { WorkflowExport } from './gen/WorkflowExport';
import type { ImportCheck } from './gen/ImportCheck';
import type { ImportRequest } from './gen/ImportRequest';
import type { ImportResult } from './gen/ImportResult';
import type { WorkflowDoc } from './gen/WorkflowDoc';
import type { WorkflowRunDetail } from './gen/WorkflowRunDetail';
import type { WorkflowTemplate } from './gen/WorkflowTemplate';
import type { WorkflowVersion } from './gen/WorkflowVersion';
import type { WorkflowPatch } from './gen/WorkflowPatch';
import type { WorkflowRun } from './gen/WorkflowRun';
import type { InboxAnswer } from './gen/InboxAnswer';
import type { InboxList } from './gen/InboxList';
import type { KindSetting } from './gen/KindSetting';
import type { NotificationList } from './gen/NotificationList';
import type { NotificationSection } from './gen/NotificationSection';
import type { ReadAll } from './gen/ReadAll';
import type { Show } from './gen/Show';
import type { NewPersona } from './gen/NewPersona';
import type { Persona } from './gen/Persona';
import type { PersonaDiff } from './gen/PersonaDiff';
import type { PersonaTemplate } from './gen/PersonaTemplate';
import type { PersonaTool } from './gen/PersonaTool';
import type { PersonaPatch } from './gen/PersonaPatch';
import type { PersonaVersion } from './gen/PersonaVersion';
import type { SavePersona } from './gen/SavePersona';
import type { NewNote } from './gen/NewNote';
import type { NoteKind } from './gen/NoteKind';
import type { NotePatch } from './gen/NotePatch';
import type { PmNote } from './gen/PmNote';
import type { PmState } from './gen/PmState';

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
  /** Saves a file opened with `baseHash` (409 `file_changed` when it changed on disk since); without it, creates one. */
  saveFile: (projectId: string, f: SaveFile) =>
    request<FileContent>(`/api/projects/${enc(projectId)}/files/content`, { method: 'PUT', body: JSON.stringify(f) }),
  renameFile: (projectId: string, r: RenameFile) =>
    request<void>(`/api/projects/${enc(projectId)}/files/rename`, { method: 'POST', body: JSON.stringify(r) }),
  deleteFile: (projectId: string, worktreeId: string | null, path: string) =>
    request<void>(`/api/projects/${enc(projectId)}/files/content?${filesQuery(worktreeId, path)}`, { method: 'DELETE' }),
  /** Tells an agent session which files the user changed (it gets it once it is free). */
  tellFilesChanged: (projectId: string, t: TellFilesChanged) =>
    request<void>(`/api/projects/${enc(projectId)}/files/tell`, { method: 'POST', body: JSON.stringify(t) }),
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
  /** A conversation's newest `last` stored events, oldest first. */
  lastSessionEvents: (id: string, last: number) =>
    request<ConversationEvent[]>(`/api/sessions/${enc(id)}/events${queryString({ last })}`),
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
  /** One Jira issue as a ticket linked to it (or the ticket that already links it). */
  importJiraIssue: (projectId: string, key: string) =>
    request<JiraImportResult>(`/api/projects/${enc(projectId)}/jira/import`, { method: 'POST', body: JSON.stringify({ key }) }),
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
    request<TicketEvent>(`/api/tickets/${enc(id)}/comments`, { method: 'POST', body: JSON.stringify({ body, by: 'user' } satisfies NewComment) }),
  /** Several parts of a parent, in order after its existing parts (PDT-38). */
  createParts: (id: string, b: NewParts) =>
    request<PartsCreated>(`/api/tickets/${enc(id)}/parts`, { method: 'POST', body: JSON.stringify(b) }),
  startTicket: (id: string, b: StartTicket) =>
    request<StartResult>(`/api/tickets/${enc(id)}/start`, { method: 'POST', body: JSON.stringify(b) }),
  checkMerge: (id: string) => request<Ticket>(`/api/tickets/${enc(id)}/check-merge`, { method: 'POST' }),
  /** Asks the project's pipeline to refine a Draft or Ready ticket (Phase 7c). */
  refineTicket: (id: string) =>
    request<Ticket>(`/api/tickets/${enc(id)}/refine`, { method: 'POST', body: JSON.stringify({ by: 'user' }) }),
  /** `ticket.finish`: Done, then everything Start made goes (or `blocked` says what stops it, changing nothing). */
  finishTicket: (id: string, body: FinishRequest) =>
    request<FinishResult>(`/api/tickets/${enc(id)}/finish`, { method: 'POST', body: JSON.stringify(body) }),
  listWorkflows: (project?: string) => request<Workflow[]>(`/api/workflows${queryString({ project })}`),
  patchWorkflow: (id: string, p: WorkflowPatch) =>
    request<Workflow>(`/api/workflows/${enc(id)}`, { method: 'PATCH', body: JSON.stringify(p) }),
  listWorkflowRuns: (q: { workflow?: string; ticket?: string; status?: string; limit?: number } = {}) =>
    request<WorkflowRun[]>(`/api/workflow-runs${queryString(q)}`),
  workflowCatalog: () => request<KindSpec[]>('/api/workflows/catalog'),
  getWorkflow: (id: string) => request<WorkflowDetail>(`/api/workflows/${enc(id)}`),
  createWorkflow: (w: NewWorkflow) => request<WorkflowDetail>('/api/workflows', { method: 'POST', body: JSON.stringify(w) }),
  /** A new version; with `propose`, a proposal waiting for review (Phase 7d). */
  saveWorkflow: (id: string, s: SaveWorkflow) =>
    request<WorkflowDetail>(`/api/workflows/${enc(id)}`, { method: 'PUT', body: JSON.stringify(s) }),
  deleteWorkflow: (id: string) => request<void>(`/api/workflows/${enc(id)}`, { method: 'DELETE' }),
  exportWorkflow: (id: string) => request<WorkflowExport>(`/api/workflows/${enc(id)}/export`),
  exportWorkflows: (ids: string[]) =>
    request<WorkflowExport>('/api/workflows/export', { method: 'POST', body: JSON.stringify({ ids }) }),
  checkImport: (text: string) =>
    request<ImportCheck>('/api/workflows/import', { method: 'POST', body: JSON.stringify({ text, check: true }) }),
  importWorkflows: (r: Omit<ImportRequest, 'check'>) =>
    request<ImportResult>('/api/workflows/import', { method: 'POST', body: JSON.stringify({ ...r, check: false }) }),
  checkWorkflow: (doc: WorkflowDoc) =>
    request<WorkflowCheck>('/api/workflows/check', { method: 'POST', body: JSON.stringify({ doc }) }),
  dryRunWorkflow: (r: DryRunRequest) =>
    request<DryRunResult>('/api/workflows/dry-run', { method: 'POST', body: JSON.stringify(r) }),
  runWorkflow: (id: string, input: unknown) =>
    request<Started>(`/api/workflows/${enc(id)}/run`, { method: 'POST', body: JSON.stringify({ input }) }),
  workflowVersions: (id: string) => request<WorkflowVersion[]>(`/api/workflows/${enc(id)}/versions`),
  workflowVersion: (id: string, v: number) => request<WorkflowVersion>(`/api/workflows/${enc(id)}/versions/${v}`),
  workflowDiff: (id: string, from: number, to: number) =>
    request<DocDiff>(`/api/workflows/${enc(id)}/diff${queryString({ from, to })}`),
  acceptWorkflowVersion: (id: string, v: number) =>
    request<WorkflowDetail>(`/api/workflows/${enc(id)}/versions/${v}/accept`, { method: 'POST' }),
  discardWorkflowVersion: (id: string, v: number) =>
    request<WorkflowDetail>(`/api/workflows/${enc(id)}/versions/${v}/discard`, { method: 'POST' }),
  getWorkflowRun: (id: string) => request<WorkflowRunDetail>(`/api/workflow-runs/${enc(id)}`),
  retryWorkflowRun: (id: string) => request<WorkflowRunDetail>(`/api/workflow-runs/${enc(id)}/retry`, { method: 'POST' }),
  resumeWorkflowRun: (id: string) => request<WorkflowRunDetail>(`/api/workflow-runs/${enc(id)}/resume`, { method: 'POST' }),
  cancelWorkflowRun: (id: string) => request<WorkflowRunDetail>(`/api/workflow-runs/${enc(id)}/cancel`, { method: 'POST' }),
  answerWorkflowStep: (run: string, step: string, choice: string) =>
    request<WorkflowRunDetail>(`/api/workflow-runs/${enc(run)}/steps/${enc(step)}/answer`, { method: 'POST', body: JSON.stringify({ choice }) }),
  listLogEvents: (q: { type?: string; limit?: number; after?: number } = {}) => request<LogEvent[]>(`/api/events${queryString(q)}`),
  listSecrets: () => request<Secret[]>('/api/secrets'),
  createSecret: (s: NewSecret) => request<Secret>('/api/secrets', { method: 'POST', body: JSON.stringify(s) }),
  patchSecret: (id: string, p: SecretPatch) =>
    request<Secret>(`/api/secrets/${enc(id)}`, { method: 'PATCH', body: JSON.stringify(p) }),
  deleteSecret: (id: string) => request<void>(`/api/secrets/${enc(id)}`, { method: 'DELETE' }),
  secretUses: (id: string) => request<SecretUse[]>(`/api/secrets/${enc(id)}/uses`),
  listConnections: () => request<Connection[]>('/api/connections'),
  connectionKinds: () => request<ConnectionKind[]>('/api/connections/kinds'),
  createConnection: (c: ConnectionInput) => request<Connection>('/api/connections', { method: 'POST', body: JSON.stringify(c) }),
  updateConnection: (id: string, c: Partial<ConnectionInput>) =>
    request<Connection>(`/api/connections/${enc(id)}`, { method: 'PATCH', body: JSON.stringify(c) }),
  deleteConnection: (id: string) => request<void>(`/api/connections/${enc(id)}`, { method: 'DELETE' }),
  setConnectionSecret: (id: string, secret: string) =>
    request<Connection>(`/api/connections/${enc(id)}/secret`, { method: 'PUT', body: JSON.stringify({ secret }) }),
  /** An MCP server's tools as last listed (null before the first listing). */
  connectionTools: (id: string) => request<McpTools | null>(`/api/connections/${enc(id)}/tools`),
  /** Lists an MCP server's tools now. */
  refreshConnectionTools: (id: string) => request<McpTools>(`/api/connections/${enc(id)}/tools`, { method: 'POST' }),
  listWorkflowTemplates: () => request<WorkflowTemplate[]>('/api/workflow-templates'),
  getWorkflowTemplate: (id: string) => request<WorkflowTemplate>(`/api/workflow-templates/${enc(id)}`),
  createWorkflowTemplate: (t: TemplateInput) =>
    request<WorkflowTemplate>('/api/workflow-templates', { method: 'POST', body: JSON.stringify(t) }),
  saveWorkflowTemplate: (id: string, t: TemplateInput) =>
    request<WorkflowTemplate>(`/api/workflow-templates/${enc(id)}`, { method: 'PUT', body: JSON.stringify(t) }),
  deleteWorkflowTemplate: (id: string) => request<void>(`/api/workflow-templates/${enc(id)}`, { method: 'DELETE' }),
  instantiateWorkflowTemplate: (id: string, i: Instantiate) =>
    request<WorkflowDetail>(`/api/workflow-templates/${enc(id)}/instantiate`, { method: 'POST', body: JSON.stringify(i) }),
  abandonAttempt: (id: string) => request<Ticket>(`/api/attempts/${enc(id)}/abandon`, { method: 'POST' }),
  approveAttempt: (id: string) => request<Ticket>(`/api/attempts/${enc(id)}/approve`, { method: 'POST' }),
  mergePreview: (id: string) => request<MergePreview>(`/api/attempts/${enc(id)}/merge`),
  /** PDT's Merge…: a person's merge (`by: user`), which the project's policy treats as its own approval. */
  mergeAttempt: (id: string, body: MergeRequest = {}) =>
    request<MergeResult>(`/api/attempts/${enc(id)}/merge`, { method: 'POST', body: JSON.stringify({ ...body, by: 'user' } satisfies MergeRequest) }),
  askAgentToFixMerge: (id: string) => request<FeedbackResult>(`/api/attempts/${enc(id)}/ask-agent`, { method: 'POST' }),
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
  /** Where a bug report would go, and the redacted diagnostics it would carry (PDT-153). */
  bugReportPreview: () => request<BugReportPreview>('/api/bug-reports/preview'),
  reportBug: (r: NewBugReport) =>
    request<BugReportFiled>('/api/bug-reports', { method: 'POST', body: JSON.stringify(r) }),
  refreshChecks: () => request<HostChecks>('/api/host/checks/refresh', { method: 'POST' }),
  /** What waits on the user: questions, approvals, stalled steps, loop limits, gates (Phase 7b). */
  inbox: (project?: string) => request<InboxList>(`/api/inbox${queryString({ project })}`),
  answerInbox: (id: string, a: InboxAnswer) =>
    request<Record<string, unknown>>(`/api/inbox/${enc(id)}/answer`, { method: 'POST', body: JSON.stringify(a) }),
  dismissInbox: (id: string) => request<void>(`/api/inbox/${enc(id)}/dismiss`, { method: 'POST' }),
  notifications: (q: { section?: NotificationSection; project?: string; unread?: boolean; limit?: number; before?: number } = {}) =>
    request<NotificationList>(`/api/notifications${queryString(q)}`),
  readNotification: (id: string) => request<void>(`/api/notifications/${enc(id)}/read`, { method: 'POST' }),
  readNotifications: (r: ReadAll) => request<void>('/api/notifications/read-all', { method: 'POST', body: JSON.stringify(r) }),
  notificationSettings: () => request<KindSetting[]>('/api/notifications/settings'),
  setNotificationSettings: (changes: Record<string, Show | null>) =>
    request<KindSetting[]>('/api/notifications/settings', { method: 'PATCH', body: JSON.stringify(changes) }),
  listPersonas: () => request<Persona[]>('/api/personas'),
  personaTools: () => request<PersonaTool[]>('/api/personas/tools'),
  createPersona: (p: NewPersona) => request<Persona>('/api/personas', { method: 'POST', body: JSON.stringify(p) }),
  /** A new version. */
  savePersona: (id: string, p: SavePersona) =>
    request<Persona>(`/api/personas/${enc(id)}`, { method: 'PUT', body: JSON.stringify(p) }),
  patchPersona: (id: string, p: PersonaPatch) =>
    request<Persona>(`/api/personas/${enc(id)}`, { method: 'PATCH', body: JSON.stringify(p) }),
  deletePersona: (id: string) => request<void>(`/api/personas/${enc(id)}`, { method: 'DELETE' }),
  personaVersions: (id: string) => request<PersonaVersion[]>(`/api/personas/${enc(id)}/versions`),
  personaDiff: (id: string, from: number, to: number) =>
    request<PersonaDiff>(`/api/personas/${enc(id)}/diff${queryString({ from, to })}`),
  personaTemplate: (id: string) => request<PersonaTemplate>(`/api/personas/${enc(id)}/template`),
  /** The project manager (PDT-81): its conversations, the open ones first (PDT-149). */
  pm: () => request<PmState>('/api/pm'),
  /** A new PM conversation beside the open ones, `text` its first message (and, without `title`, its title). */
  startPm: (text?: string, title?: string) =>
    request<Session>('/api/pm/sessions', { method: 'POST', body: JSON.stringify({ text, title }) }),
  renamePm: (id: string, title: string) =>
    request<Session>(`/api/pm/sessions/${enc(id)}`, { method: 'PATCH', body: JSON.stringify({ title }) }),
  /** Closes a PM conversation: it writes its journal, then ends. */
  closePm: (id: string) => request<void>(`/api/pm/sessions/${enc(id)}`, { method: 'DELETE' }),
  pmNotes: (q: { q?: string; project?: string; kind?: NoteKind; limit?: number } = {}) => request<PmNote[]>(`/api/pm/notes${queryString(q)}`),
  pmNote: (id: string) => request<PmNote>(`/api/pm/notes/${enc(id)}`),
  createPmNote: (n: NewNote) => request<PmNote>('/api/pm/notes', { method: 'POST', body: JSON.stringify(n) }),
  patchPmNote: (id: string, p: NotePatch) =>
    request<PmNote>(`/api/pm/notes/${enc(id)}`, { method: 'PATCH', body: JSON.stringify(p) }),
  deletePmNote: (id: string) => request<void>(`/api/pm/notes/${enc(id)}`, { method: 'DELETE' }),
  /** The user's todo list (PDT-125): open ones in their order, then done ones (latest first). */
  todos: {
    list: (q: { project?: string; done?: boolean } = {}) => request<Todo[]>(`/api/todos${queryString(q)}`),
    create: (t: NewTodo) => request<Todo>('/api/todos', { method: 'POST', body: JSON.stringify(t) }),
    patch: (id: string, p: TodoPatch) => request<Todo>(`/api/todos/${enc(id)}`, { method: 'PATCH', body: JSON.stringify(p) }),
    /** `series` (a repeating todo): `only` skips to the next one, `series` stops it repeating. */
    remove: (id: string, series?: 'only' | 'series') => request<void>(`/api/todos/${enc(id)}${queryString({ series })}`, { method: 'DELETE' }),
  },
};
