import { describe, expect, it } from 'vitest';
import { linesOf, relocate, threadsOf } from './anchors';
import type { ReviewComment } from '../api/gen/ReviewComment';

const c = (over: Partial<ReviewComment>): ReviewComment => ({
  id: 'c', worktreeId: 'w', parentId: null, path: 'a.ts', side: 'new', line: 2, endLine: 2, lineText: 'b', body: 'x',
  author: 'user', sentAt: null, sessionId: null, resolvedAt: null, createdAt: 1, updatedAt: 1, ...over,
});

describe('relocate', () => {
  const file = ['a', 'b', 'c', 'b', 'd'];

  it('keeps lines that are still where they were', () => {
    expect(relocate({ line: 2, endLine: 2, lineText: 'b' }, file)).toBe(2);
    expect(relocate({ line: 2, endLine: 3, lineText: 'b\nc' }, file)).toBe(2);
  });

  it('follows lines that moved, to the nearest copy', () => {
    expect(relocate({ line: 1, endLine: 2, lineText: 'c\nb' }, file)).toBe(3);
    expect(relocate({ line: 5, endLine: 5, lineText: 'b' }, file)).toBe(4);
    expect(relocate({ line: 1, endLine: 1, lineText: 'b' }, file)).toBe(2);
  });

  it('gives up when the lines are gone', () => {
    expect(relocate({ line: 2, endLine: 2, lineText: 'zzz' }, file)).toBeNull();
    expect(relocate({ line: 9, endLine: 9, lineText: '' }, file)).toBeNull();
  });
});

describe('linesOf', () => {
  it('splits text into lines without a phantom last one', () => {
    expect(linesOf('a\nb\n')).toEqual(['a', 'b']);
    expect(linesOf('a\r\nb')).toEqual(['a', 'b']);
    expect(linesOf('')).toEqual([]);
  });
});

describe('threadsOf', () => {
  it('groups replies under their roots, per file, in line order', () => {
    const list = [
      c({ id: 'r2', path: 'b.ts', line: 1 }),
      c({ id: 'r1', line: 5, endLine: 5 }),
      c({ id: 'x', parentId: 'r1', createdAt: 3 }),
      c({ id: 'r0', line: 1, endLine: 1, side: 'old' }),
    ];
    const t = threadsOf(list);
    expect(t.map((th) => [th.root.id, th.replies.map((r) => r.id)])).toEqual([['r0', []], ['r1', ['x']], ['r2', []]]);
    expect(t[1]!.drafts).toBe(2);
  });
});
