import { enc, queryString, request } from './core';
import type { Changes } from './gen/Changes';
import type { FeedbackResult } from './gen/FeedbackResult';
import type { FileSides } from './gen/FileSides';
import type { MergePreview } from './gen/MergePreview';
import type { MergeRequest } from './gen/MergeRequest';
import type { MergeResult } from './gen/MergeResult';
import type { NewReviewComment } from './gen/NewReviewComment';
import type { ReviewComment } from './gen/ReviewComment';
import type { ReviewCommentPatch } from './gen/ReviewCommentPatch';
import type { Ticket } from './gen/Ticket';

/** Reviewing an attempt's changes: diffs, review comments, feedback, approve and merge. */
export const reviewApi = {
  abandonAttempt: (id: string) => request<Ticket>(`/api/attempts/${enc(id)}/abandon`, { method: 'POST' }),
  approveAttempt: (id: string) => request<Ticket>(`/api/attempts/${enc(id)}/approve`, { method: 'POST' }),
  mergePreview: (id: string) => request<MergePreview>(`/api/attempts/${enc(id)}/merge`),
  /** PDT's Merge…: a person's merge (`by: user`), which the project's policy treats as its own approval. */
  mergeAttempt: (id: string, body: MergeRequest = {}) =>
    request<MergeResult>(`/api/attempts/${enc(id)}/merge`, { method: 'POST', body: JSON.stringify({ ...body, by: 'user' } satisfies MergeRequest) }),
  askAgentToFixMerge: (id: string) => request<FeedbackResult>(`/api/attempts/${enc(id)}/ask-agent`, { method: 'POST' }),
  getChanges: (worktreeId: string) => request<Changes>(`/api/worktrees/${enc(worktreeId)}/changes`),
  getFileSides: (worktreeId: string, path: string, oldPath?: string | null) =>
    request<FileSides>(`/api/worktrees/${enc(worktreeId)}/changes/file${queryString({ path, oldPath })}`),
  listReviewComments: (worktreeId: string) => request<ReviewComment[]>(`/api/worktrees/${enc(worktreeId)}/comments`),
  createReviewComment: (worktreeId: string, c: NewReviewComment) =>
    request<ReviewComment>(`/api/worktrees/${enc(worktreeId)}/comments`, { method: 'POST', body: JSON.stringify(c) }),
  updateReviewComment: (id: string, p: ReviewCommentPatch) =>
    request<ReviewComment>(`/api/review-comments/${enc(id)}`, { method: 'PATCH', body: JSON.stringify(p) }),
  deleteReviewComment: (id: string) => request<void>(`/api/review-comments/${enc(id)}`, { method: 'DELETE' }),
  sendFeedback: (worktreeId: string) => request<FeedbackResult>(`/api/worktrees/${enc(worktreeId)}/feedback`, { method: 'POST' }),
};
