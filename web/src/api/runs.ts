import { enc, queryString, request } from './core';
import type { NewRun } from './gen/NewRun';
import type { ProjectCommands } from './gen/ProjectCommands';
import type { Run } from './gen/Run';
import type { RunConfigView } from './gen/RunConfigView';

/** A project's runs (`.pdt/project.toml`) and its run configuration. */
export const runsApi = {
  /** A project's runs; with `ended`, the active ones, those a tab holds and only the `ended` most recently finished. */
  listRuns: (projectId: string, q: { ended?: number } = {}) =>
    request<Run[]>(`/api/projects/${enc(projectId)}/runs${queryString(q)}`),
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
  /** The commands of a checkout's run configuration, and which wait on the user's approval. */
  projectCommands: (projectId: string, worktreeId: string | null) =>
    request<ProjectCommands>(
      `/api/projects/${enc(projectId)}/commands${worktreeId ? `?worktreeId=${enc(worktreeId)}` : ''}`,
    ),
  /** The user approves these commands (as the user: an agent can't). */
  trustCommands: (projectId: string, fingerprints: string[]) =>
    request<void>(`/api/projects/${enc(projectId)}/commands/trust`, { method: 'POST', body: JSON.stringify({ fingerprints }) }, { asUser: true }),
};
