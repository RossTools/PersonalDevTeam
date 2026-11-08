// A checkout's files: its folders' entries, a file's content, a search in it, and the palette's file finder.
import type { FileSearchQuery } from '../api/files';

export const fileKeys = {
  /** Every folder listed in a checkout (a write there refreshes them). */
  folders: (projectId: string, worktreeId: string | null) => ['files', projectId, worktreeId] as const,
  folder: (projectId: string, worktreeId: string | null, path: string) => ['files', projectId, worktreeId, path] as const,
  /** Every file read in a checkout. */
  contents: (projectId: string, worktreeId: string | null) => ['file', projectId, worktreeId] as const,
  content: (projectId: string, worktreeId: string | null, path: string | null) => ['file', projectId, worktreeId, path] as const,
  search: (projectId: string, worktreeId: string | null, q: FileSearchQuery) => ['fileSearch', projectId, worktreeId, q] as const,
  /** The palette's files by name. */
  find: (projectId: string, worktreeId: string | null, q: string) => ['files', 'find', projectId, worktreeId, q] as const,
};
