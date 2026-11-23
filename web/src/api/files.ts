import { enc, request } from './core';
import type { FileContent } from './gen/FileContent';
import type { FileEntry } from './gen/FileEntry';
import type { RenameFile } from './gen/RenameFile';
import type { SaveFile } from './gen/SaveFile';
import type { SearchResult } from './gen/SearchResult';
import type { TellFilesChanged } from './gen/TellFilesChanged';

/**
 * Which checkout of a project: its own (no `worktreeId`), one of its worktrees, or (`instanceId`) the project's
 * checkout on a joined remote instance.
 */
export interface Checkout {
  worktreeId: string | null;
  instanceId?: string | null;
}

/** The checkout's query parameters (`worktreeId` or `instanceId`), for a URL or a request body. */
export function checkoutParams(at: Checkout): { worktreeId?: string; instanceId?: string } {
  if (at.instanceId) return { instanceId: at.instanceId };
  return at.worktreeId ? { worktreeId: at.worktreeId } : {};
}

function filesQuery(at: Checkout, path: string): string {
  return new URLSearchParams({ ...checkoutParams(at), path }).toString();
}

/** How a download goes: `zip` sends a folder as a `.zip`, `ignored` puts the files `.gitignore` leaves out in it. */
export interface DownloadOptions {
  zip?: boolean;
  ignored?: boolean;
}

function downloadQuery(at: Checkout, path: string, o: DownloadOptions, check = false): string {
  const params = new URLSearchParams(filesQuery(at, path));
  if (o.zip) params.set('zip', '1');
  if (o.ignored) params.set('ignored', '1');
  if (check) params.set('check', '1');
  return params.toString();
}

/** What to search the files for: the text (a regular expression with `regex`), and the files to look in (`glob`). */
export interface FileSearchQuery {
  q: string;
  case: boolean;
  word: boolean;
  regex: boolean;
  glob: string;
}

/** A checkout's files (`at`: the project's, a worktree's or a remote instance's): browse, read, save, rename, delete, search, find by name. */
export const filesApi = {
  listFiles: (projectId: string, at: Checkout, path: string) =>
    request<FileEntry[]>(`/api/projects/${enc(projectId)}/files?${filesQuery(at, path)}`),
  fileContent: (projectId: string, at: Checkout, path: string) =>
    request<FileContent>(`/api/projects/${enc(projectId)}/files/content?${filesQuery(at, path)}`),
  /** Saves a file opened with `baseHash` (409 `file_changed` when it changed on disk since); without it, creates one. */
  saveFile: (projectId: string, f: SaveFile) =>
    request<FileContent>(`/api/projects/${enc(projectId)}/files/content`, { method: 'PUT', body: JSON.stringify(f) }),
  renameFile: (projectId: string, r: RenameFile) =>
    request<void>(`/api/projects/${enc(projectId)}/files/rename`, { method: 'POST', body: JSON.stringify(r) }),
  deleteFile: (projectId: string, at: Checkout, path: string) =>
    request<void>(`/api/projects/${enc(projectId)}/files/content?${filesQuery(at, path)}`, { method: 'DELETE' }),
  /** The files whose text matches, grouped by file (capped; `timedOut` when the search ran out of time). */
  searchFiles: (projectId: string, at: Checkout, s: FileSearchQuery, signal?: AbortSignal) => {
    const params = new URLSearchParams({ ...checkoutParams(at), q: s.q });
    for (const flag of ['case', 'word', 'regex'] as const) if (s[flag]) params.set(flag, 'true');
    if (s.glob.trim()) params.set('glob', s.glob.trim());
    return request<SearchResult>(`/api/projects/${enc(projectId)}/files/search?${params}`, { signal });
  },
  /** The checkout's files whose names best match `q`, best first (at most 20; none in a plain folder). */
  findFiles: (projectId: string, at: Checkout, q: string, signal?: AbortSignal) =>
    request<string[]>(`/api/projects/${enc(projectId)}/files/find?${new URLSearchParams({ ...checkoutParams(at), q })}`, { signal }),
  /** Where a file (or, with `zip`, a folder) downloads from: pdtd streams it with its name. */
  fileDownloadUrl: (projectId: string, at: Checkout, path: string, o: DownloadOptions = {}) =>
    `/api/projects/${enc(projectId)}/files/download?${downloadQuery(at, path, o)}`,
  /** Fails with why a download would (a folder over the size cap, a file gone), before the browser saves an error. */
  checkFileDownload: (projectId: string, at: Checkout, path: string, o: DownloadOptions = {}) =>
    request<void>(`/api/projects/${enc(projectId)}/files/download?${downloadQuery(at, path, o, true)}`),
  /** Tells an agent session which files the user changed (it gets it once it is free). */
  tellFilesChanged: (projectId: string, t: TellFilesChanged) =>
    request<void>(`/api/projects/${enc(projectId)}/files/tell`, { method: 'POST', body: JSON.stringify(t) }),
};
