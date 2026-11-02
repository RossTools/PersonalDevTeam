import { builtinSettingsApi } from './builtinSettings';
import { enc, queryString, request } from './core';
import type { CleanAllResult } from './gen/CleanAllResult';
import type { CleanResult } from './gen/CleanResult';
import type { StorageSettings } from './gen/StorageSettings';
import type { StorageView } from './gen/StorageView';

/** Disk use and cleaning up worktrees. */
export const storageApi = {
  storage: (project?: string) => request<StorageView>(`/api/storage${queryString({ project })}`),
  refreshStorage: () => request<void>('/api/storage/refresh', { method: 'POST' }),
  cleanWorktree: (id: string) => request<CleanResult>(`/api/worktrees/${enc(id)}/clean`, { method: 'POST' }),
  cleanFinished: (project?: string) =>
    request<CleanAllResult>(`/api/storage/clean-finished${queryString({ project })}`, { method: 'POST' }),
  putProjectStorage: (projectId: string, s: StorageSettings) =>
    request<StorageSettings>(`/api/projects/${enc(projectId)}/storage`, { method: 'PUT', body: JSON.stringify(s) }),
  /** The built-in Clean up storage's inputs: the thresholds and what happens when a disk is almost full. */
  storageCleanup: builtinSettingsApi('/api/storage/cleanup'),
  /** Resumes pickup, whoever paused it. */
  resumePickup: () => request<void>('/api/pickup/pause', { method: 'DELETE' }),
};
