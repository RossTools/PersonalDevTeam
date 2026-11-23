// A checkout's files: its folders' entries, a file's content, a search in it, and the palette's file finder.
import type { Checkout, FileSearchQuery } from '../api/files';

/** The checkout's part of a key: a worktree, or a remote instance's checkout (`@<instance>`). */
const where = (at: Checkout) => (at.instanceId ? `@${at.instanceId}` : at.worktreeId);

export const fileKeys = {
  /** Every folder listed in a checkout (a write there refreshes them). */
  folders: (projectId: string, at: Checkout) => ['files', projectId, where(at)] as const,
  folder: (projectId: string, at: Checkout, path: string) => ['files', projectId, where(at), path] as const,
  /** Every file read in a checkout. */
  contents: (projectId: string, at: Checkout) => ['file', projectId, where(at)] as const,
  content: (projectId: string, at: Checkout, path: string | null) => ['file', projectId, where(at), path] as const,
  search: (projectId: string, at: Checkout, q: FileSearchQuery) => ['fileSearch', projectId, where(at), q] as const,
  /** The palette's files by name. */
  find: (projectId: string, at: Checkout, q: string) => ['files', 'find', projectId, where(at), q] as const,
};
