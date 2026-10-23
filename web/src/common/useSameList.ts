import { useRef } from 'react';

/** The same items, in the same order, by reference. */
export function sameItems<T>(a: readonly T[], b: readonly T[]): boolean {
  return a.length === b.length && a.every((x, i) => x === b[i]);
}

/**
 * `list`, or the previous render's list when it holds the same items (by reference). Polled query data keeps its
 * identity while the server's answer is unchanged (TanStack's structural sharing), so `useSameList(qs.map((q) =>
 * q.data))` is a memo key that changes only when some query's data did, never because a poll merely ran (PDT-136).
 */
export function useSameList<T>(list: T[]): T[] {
  const prev = useRef(list);
  if (!sameItems(prev.current, list)) prev.current = list;
  return prev.current;
}
