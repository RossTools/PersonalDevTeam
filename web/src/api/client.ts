import type { ApiErrorBody } from './gen/ApiErrorBody';
import type { Health } from './gen/Health';
import type { NewProject } from './gen/NewProject';
import type { NewSession } from './gen/NewSession';
import type { NewWorktree } from './gen/NewWorktree';
import type { Project } from './gen/Project';
import type { Session } from './gen/Session';
import type { TerminalSize } from './gen/TerminalSize';
import type { Worktree } from './gen/Worktree';

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

export const api = {
  health: () => request<Health>('/api/health'),
  listProjects: () => request<Project[]>('/api/projects'),
  createProject: (p: NewProject) => request<Project>('/api/projects', { method: 'POST', body: JSON.stringify(p) }),
  listWorktrees: (projectId: string) => request<Worktree[]>(`/api/projects/${enc(projectId)}/worktrees`),
  createWorktree: (projectId: string, w: NewWorktree) =>
    request<Worktree>(`/api/projects/${enc(projectId)}/worktrees`, { method: 'POST', body: JSON.stringify(w) }),
  listSessions: (projectId: string) => request<Session[]>(`/api/projects/${enc(projectId)}/sessions`),
  createSession: (s: NewSession) => request<Session>('/api/sessions', { method: 'POST', body: JSON.stringify(s) }),
  killSession: (id: string) => request<void>(`/api/sessions/${enc(id)}/kill`, { method: 'POST' }),
  restartSession: (id: string, size: TerminalSize) =>
    request<Session>(`/api/sessions/${enc(id)}/restart`, { method: 'POST', body: JSON.stringify(size) }),
  renameSession: (id: string, title: string) =>
    request<Session>(`/api/sessions/${enc(id)}`, { method: 'PATCH', body: JSON.stringify({ title }) }),
  deleteSession: (id: string) => request<void>(`/api/sessions/${enc(id)}`, { method: 'DELETE' }),
};
