import { useCallback, useState } from 'react';
import type { ChangedFile } from '../api/gen/ChangedFile';

/** Per browser: the version of each file marked Viewed, per worktree. A file that changes since is unviewed. */
const key = (worktreeId: string) => `pdt.review.viewed.${worktreeId}`;

function load(worktreeId: string): Record<string, string> {
  try {
    const v = JSON.parse(localStorage.getItem(key(worktreeId)) ?? '{}');
    return v && typeof v === 'object' ? v : {};
  } catch {
    return {};
  }
}

export function useViewed(worktreeId: string) {
  const [marks, setMarks] = useState(() => load(worktreeId));
  const isViewed = useCallback((f: ChangedFile) => marks[f.path] === f.version, [marks]);
  const setViewed = useCallback(
    (f: ChangedFile, on: boolean) => {
      setMarks((m) => {
        const next = { ...m };
        if (on) next[f.path] = f.version;
        else delete next[f.path];
        try {
          localStorage.setItem(key(worktreeId), JSON.stringify(next));
        } catch {
          // Storage blocked: the marks last for this page.
        }
        return next;
      });
    },
    [worktreeId],
  );
  return { isViewed, setViewed };
}
