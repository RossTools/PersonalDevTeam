import { enc, request } from './core';
import type { FileContent } from './gen/FileContent';
import type { FileEntry } from './gen/FileEntry';
import type { RenameFile } from './gen/RenameFile';
import type { SaveFile } from './gen/SaveFile';
import type { SearchResult } from './gen/SearchResult';
import type { TellFilesChanged } from './gen/TellFilesChanged';

function filesQuery(worktreeId: string | null, path: string): string {
  return new URLSearchParams({ ...(worktreeId ? { worktreeId } : {}), path }).toString();
}

/** What to search the files for: the text (a regular expression with `regex`), and the files to look in (`glob`). */
export interface FileSearchQuery {
  q: string;
  case: boolean;
  word: boolean;
  regex: boolean;
  glob: string;
}

/** A project's (or worktree's) files: browse, read, save, rename, delete, search. */
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
  /** The files whose text matches, grouped by file (capped; `timedOut` when the search ran out of time). */
  searchFiles: (projectId: string, worktreeId: string | null, s: FileSearchQuery, signal?: AbortSignal) => {
    const params = new URLSearchParams({ ...(worktreeId ? { worktreeId } : {}), q: s.q });
    for (const flag of ['case', 'word', 'regex'] as const) if (s[flag]) params.set(flag, 'true');
    if (s.glob.trim()) params.set('glob', s.glob.trim());
    return request<SearchResult>(`/api/projects/${enc(projectId)}/files/search?${params}`, { signal });
  },
  /** Tells an agent session which files the user changed (it gets it once it is free). */
  tellFilesChanged: (projectId: string, t: TellFilesChanged) =>
    request<void>(`/api/projects/${enc(projectId)}/files/tell`, { method: 'POST', body: JSON.stringify(t) }),
};
