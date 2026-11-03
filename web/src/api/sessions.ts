import { enc, queryString, request } from './core';
import type { AgentCapabilities } from './gen/AgentCapabilities';
import type { AgentKind } from './gen/AgentKind';
import type { Answer } from './gen/Answer';
import type { AnswerRequest } from './gen/AnswerRequest';
import type { Attachment } from './gen/Attachment';
import type { ConversationEvent } from './gen/ConversationEvent';
import type { NewSession } from './gen/NewSession';
import type { PendingMessage } from './gen/PendingMessage';
import type { PermissionMode } from './gen/PermissionMode';
import type { SendMessage } from './gen/SendMessage';
import type { Session } from './gen/Session';
import type { SessionMode } from './gen/SessionMode';
import type { TerminalSize } from './gen/TerminalSize';
import type { TicketDraft } from './gen/TicketDraft';

/** `PATCH /api/sessions/{id}`: any of these; `model: null` goes back to the agent's default. */
export interface SessionPatch {
  title?: string;
  model?: string | null;
  permissionMode?: PermissionMode;
}

/** Sessions: terminals and agent conversations. */
export const sessionsApi = {
  getSession: (id: string) => request<Session>(`/api/sessions/${enc(id)}`),
  /** A project's sessions; with `ended`, the running ones, those a tab holds and only the `ended` most recently ended. */
  listSessions: (projectId: string, q: { ended?: number } = {}) =>
    request<Session[]>(`/api/projects/${enc(projectId)}/sessions${queryString(q)}`),
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
    }, { asUser: true }),
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
};
