import { useEffect, useRef, useState } from 'react';
import type { Project } from '../api/gen/Project';
import type { Place } from '../common/PlacePicker';
import { whenAllowed } from '../shell/router';
import type { CodeTarget } from './CodeViewer';

// The Files view's state: the checkout, the open folders and the file shown. The shell holds it from the first paint;
// the view itself (FolderView) loads when Files is first opened.

/**
 * The checkout picked and the file shown, when something else holds them (the app keeps them in its route, PDT-111);
 * without it the view keeps its own.
 */
export interface FolderSelection {
  picked: Place | null;
  file: string | null;
  onChange: (picked: Place | null, file: string | null) => void;
}

export function useFolderView(projects: Project[], initial: Place | null, held?: FolderSelection) {
  const [ownPicked, setOwnPicked] = useState<Place | null>(null);
  const [ownFile, setOwnFile] = useState<string | null>(null);
  const picked = held ? held.picked : ownPicked;
  const file = held ? held.file : ownFile;
  // Held: the app's navigation asks about unsaved edits; here, the same guard does.
  const change = (p: Place | null, f: string | null) => {
    if (held) held.onChange(p, f);
    else whenAllowed(() => { setOwnPicked(p); setOwnFile(f); });
  };
  const place = picked ?? initial ?? (projects[0] ? { projectId: projects[0].id, worktreeId: null } : null);
  const placeKey = place ? `${place.projectId}:${place.worktreeId}` : '';
  const [open, setOpen] = useState<Set<string>>(new Set());
  // The file with unsaved edits, if any: it can't be renamed or deleted under the editor.
  const [unsaved, setUnsaved] = useState<string | null>(null);
  // Where a search result opens its file: the viewer scrolls to it and marks the match (`n` tells clicks apart).
  const [at, setAt] = useState<(CodeTarget & { path: string }) | null>(null);
  // Another checkout starts with its folders closed (not the first one shown once projects load: a linked file's
  // folders are open in it).
  const shownPlace = useRef(placeKey);
  useEffect(() => {
    const was = shownPlace.current;
    shownPlace.current = placeKey;
    if (was && was !== placeKey) setOpen(new Set());
  }, [placeKey]);
  // A file shown from a link or a reload: its folders open, so it is in the tree.
  useEffect(() => {
    const parents = file?.split('/').slice(0, -1).map((_, i, all) => all.slice(0, i + 1).join('/')) ?? [];
    if (parents.some((p) => !open.has(p))) setOpen((s) => new Set([...s, ...parents]));
    // Only when the file changes: folders closed by hand stay closed.
  }, [file]);
  return {
    projects,
    place,
    pick: (p: Place) => change(p, null),
    open,
    setOpen,
    toggle: (path: string) => setOpen((s) => { const n = new Set(s); if (n.has(path)) n.delete(path); else n.add(path); return n; }),
    file,
    setFile: (f: string | null) => change(picked, f),
    at,
    /** Opens `path` at `line`, the match `[from, to)` in it marked (UTF-16 units). */
    openAt: (path: string, line: number, from: number, to: number) => {
      setAt((was) => ({ path, line, from, to, n: (was?.n ?? 0) + 1 }));
      change(picked, path);
    },
    unsaved,
    setUnsaved,
  };
}

export type FolderViewState = ReturnType<typeof useFolderView>;
