import type { ReviewComment } from '../api/gen/ReviewComment';

/** A file's text as lines (no trailing empty line for a final newline; CRLF too). */
export function linesOf(text: string): string[] {
  if (!text) return [];
  const lines = text.split(/\r?\n/);
  if (lines[lines.length - 1] === '') lines.pop();
  return lines;
}

/**
 * Where a comment's lines are now (1-based first line), given the side's current lines: where they were if they are
 * still there, else the nearest exact copy of them, else null (outdated).
 */
export function relocate(anchor: { line: number; endLine: number; lineText: string }, lines: string[]): number | null {
  const want = anchor.lineText.split(/\r?\n/);
  const n = Math.max(1, anchor.endLine - anchor.line + 1);
  const block = want.length === n ? want : null;
  const matches = (at: number) => block !== null && at >= 0 && at + n <= lines.length && block.every((l, i) => lines[at + i] === l);
  const at = anchor.line - 1;
  if (matches(at)) return anchor.line;
  let best: number | null = null;
  for (let i = 0; i + n <= lines.length; i++) {
    if (matches(i) && (best === null || Math.abs(i - at) < Math.abs(best - at))) best = i;
  }
  return best === null ? null : best + 1;
}

export interface Thread {
  root: ReviewComment;
  replies: ReviewComment[];
  /** Drafts in the thread (root and replies). */
  drafts: number;
}

/** A worktree's comments as threads, by file then line. */
export function threadsOf(comments: ReviewComment[]): Thread[] {
  const roots = comments.filter((c) => !c.parentId);
  return roots
    .map((root) => {
      const replies = comments.filter((c) => c.parentId === root.id).sort((a, b) => a.createdAt - b.createdAt);
      const drafts = [root, ...replies].filter((c) => c.sentAt === null).length;
      return { root, replies, drafts };
    })
    .sort((a, b) => a.root.path.localeCompare(b.root.path) || a.root.line - b.root.line || a.root.createdAt - b.root.createdAt);
}

/** "a.ts:3" or "a.ts:3-5". */
export function anchorLabel(c: Pick<ReviewComment, 'path' | 'line' | 'endLine'>): string {
  return `${c.path}:${c.endLine > c.line ? `${c.line}-${c.endLine}` : c.line}`;
}
