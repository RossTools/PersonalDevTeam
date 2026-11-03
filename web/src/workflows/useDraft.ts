// The builder's draft: one document the three views edit, with undo, the server's check of it,
// and what happened to the saved version meanwhile. Polling never replaces a dirty draft.
import { useCallback, useEffect, useRef, useState } from 'react';
import { keepPreviousData, useQuery } from '@tanstack/react-query';
import { api } from '../api/client';
import type { WorkflowDoc } from '../api/gen/WorkflowDoc';
import { sameDoc } from './textFormat';
import { workflowKeys } from './queries';

function useDebounced<T>(value: T, ms: number): T {
  const [v, setV] = useState(value);
  useEffect(() => {
    const t = setTimeout(() => setV(value), ms);
    return () => clearTimeout(t);
  }, [value, ms]);
  return v;
}

export interface Draft {
  doc: WorkflowDoc;
  set: (doc: WorkflowDoc) => void;
  dirty: boolean;
  /** The saved version changed while the draft was dirty. */
  savedElsewhere: boolean;
  /** Back to the saved version. */
  discard: () => void;
  undo: () => void;
  redo: () => void;
  canUndo: boolean;
  canRedo: boolean;
}

export function useDraft(saved: WorkflowDoc, savedKey: string): Draft {
  const [doc, setDoc] = useState(saved);
  const [base, setBase] = useState({ key: savedKey, doc: saved });
  const past = useRef<WorkflowDoc[]>([]);
  const future = useRef<WorkflowDoc[]>([]);
  const [, bump] = useState(0);
  const dirty = !sameDoc(doc, base.doc);
  const [elsewhere, setElsewhere] = useState(false);
  useEffect(() => {
    if (savedKey === base.key) return;
    if (dirty && !sameDoc(doc, saved)) {
      setElsewhere(true);
      return;
    }
    setBase({ key: savedKey, doc: saved });
    setDoc(saved);
    setElsewhere(false);
    past.current = [];
    future.current = [];
  }, [savedKey, saved, base.key, dirty, doc]);
  const set = useCallback((next: WorkflowDoc) => {
    setDoc((cur) => {
      if (next === cur) return cur;
      past.current = [...past.current.slice(-99), cur];
      future.current = [];
      return next;
    });
    bump((n) => n + 1);
  }, []);
  const discard = () => {
    setBase({ key: savedKey, doc: saved });
    setDoc(saved);
    setElsewhere(false);
    past.current = [];
    future.current = [];
  };
  const undo = () => {
    const prev = past.current.at(-1);
    if (!prev) return;
    past.current = past.current.slice(0, -1);
    future.current = [doc, ...future.current];
    setDoc(prev);
  };
  const redo = () => {
    const next = future.current[0];
    if (!next) return;
    future.current = future.current.slice(1);
    past.current = [...past.current, doc];
    setDoc(next);
  };
  return { doc, set, dirty, savedElsewhere: elsewhere, discard, undo, redo, canUndo: past.current.length > 0, canRedo: future.current.length > 0 };
}

/** The server's check of a document (issues and each node's ports), a moment after it stops changing. */
export function useCheck(doc: WorkflowDoc, enabled = true) {
  const text = useDebounced(JSON.stringify(doc), 250);
  return useQuery({
    queryKey: workflowKeys.check(text),
    queryFn: () => api.checkWorkflow(JSON.parse(text) as WorkflowDoc),
    enabled,
    placeholderData: keepPreviousData,
    staleTime: Infinity,
  });
}
