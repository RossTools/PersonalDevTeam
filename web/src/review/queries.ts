import { useQuery, type QueryClient } from '@tanstack/react-query';
import { api } from '../api/client';
import type { ChangedFile } from '../api/gen/ChangedFile';

/** The list of changed files is re-read this often (it runs git). */
const CHANGES_POLL = 5000;

export const changesKey = (worktreeId: string) => ['changes', worktreeId] as const;
export const commentsKey = (worktreeId: string) => ['review-comments', worktreeId] as const;

export function useChanges(worktreeId: string | null) {
  return useQuery({
    queryKey: changesKey(worktreeId ?? ''),
    queryFn: () => api.getChanges(worktreeId!),
    enabled: !!worktreeId,
    refetchInterval: CHANGES_POLL,
  });
}

/** Lines added and removed across a worktree's changes (null until known, or without a worktree). */
export function useDiffSize(worktreeId: string | null): { added: number; removed: number; files: number } | null {
  const q = useChanges(worktreeId);
  if (!q.data) return null;
  const files = q.data.files;
  return { added: files.reduce((n, f) => n + f.added, 0), removed: files.reduce((n, f) => n + f.removed, 0), files: files.length };
}

/** A file's two sides, read again only when its version changes. */
export function useFileSides(worktreeId: string, file: ChangedFile | null) {
  return useQuery({
    queryKey: ['file-sides', worktreeId, file?.path, file?.version],
    queryFn: () => api.getFileSides(worktreeId, file!.path, file!.oldPath),
    enabled: !!file && !file.binary,
    staleTime: Infinity,
  });
}

export function useReviewComments(worktreeId: string) {
  return useQuery({ queryKey: commentsKey(worktreeId), queryFn: () => api.listReviewComments(worktreeId), refetchInterval: CHANGES_POLL });
}

export function invalidateReview(qc: QueryClient, worktreeId: string) {
  void qc.invalidateQueries({ queryKey: commentsKey(worktreeId) });
  void qc.invalidateQueries({ queryKey: changesKey(worktreeId) });
}
