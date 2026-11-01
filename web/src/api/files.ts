import { enc, request } from './core';
import type { FileContent } from './gen/FileContent';
import type { FileEntry } from './gen/FileEntry';
import type { RenameFile } from './gen/RenameFile';
import type { SaveFile } from './gen/SaveFile';
import type { TellFilesChanged } from './gen/TellFilesChanged';

function filesQuery(worktreeId: string | null, path: string): string {
  return new URLSearchParams({ ...(worktreeId ? { worktreeId } : {}), path }).toString();
}

/** A project's (or worktree's) files: browse, read, save, rename, delete. */
export const filesApi = {
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
};
