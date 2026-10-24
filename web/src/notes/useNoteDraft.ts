import { useCallback, useEffect, useRef, useState } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import { toast } from '@ui';
import { api } from '../api/client';
import type { PmNote } from '../api/gen/PmNote';
import { actionError } from '../common/useAction';
import { useRefreshPm } from '../pm/usePm';
import { titleFor, type Whose } from './model';
import { useNote } from './useNotes';

/** How long typing has to stop before the note saves. */
export const AUTOSAVE_MS = 800;

/** `saved`: the server has what is shown; `pending`: edited, saving soon; `saving`; `error`: the last save failed. */
export type SaveState = 'saved' | 'pending' | 'saving' | 'error';

/** What a note not created yet is created with (read when its first save runs). */
export interface NewNoteOptions {
  kind: Whose;
  /** A project id; null: host-wide. */
  project: string | null;
  pinned: boolean;
  /** Called once the first save has created it, with its id. */
  onCreated: (id: string) => void;
}

interface Text {
  title: string;
  body: string;
}

/**
 * A note's title and body as edited here, saved by themselves (PDT-143): AUTOSAVE_MS after typing stops, on `flush`
 * (blur, Esc) and when the editor goes away. The text comes from the server once per note; after that the polled copy
 * only replaces it when it changed elsewhere and nothing here is unsaved, so a poll never overwrites an edit. A note
 * that doesn't exist yet (`initialId` null) is created by its first save, and only once something is written.
 */
export function useNoteDraft(initialId: string | null, newNote: NewNoteOptions) {
  const qc = useQueryClient();
  const refresh = useRefreshPm();
  const [noteId, setNoteId] = useState(initialId);
  const one = useNote(noteId);
  const server = one.data;
  const [text, setText] = useState<Text>({ title: '', body: '' });
  const [ready, setReady] = useState(initialId === null);
  const [state, setState] = useState<SaveState>('saved');

  const idRef = useRef(initialId);
  const textRef = useRef(text);
  textRef.current = text;
  // The text (as edited here) the server has; and the server's own copy of it (the title may have been filled in).
  const baseline = useRef<Text>({ title: '', body: '' });
  const sent = useRef<Text | null>(null);
  const readyRef = useRef(ready);
  const inFlight = useRef(false);
  const again = useRef(false);
  const stopped = useRef(false);
  const mounted = useRef(true);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const newRef = useRef(newNote);
  newRef.current = newNote;

  const dirty = () => textRef.current.title !== baseline.current.title || textRef.current.body !== baseline.current.body;

  // The server's copy: the first one fills the editor; later ones only when changed elsewhere and nothing is unsaved.
  useEffect(() => {
    if (!server) return;
    const theirs = { title: server.title, body: server.body };
    if (!readyRef.current) {
      readyRef.current = true;
      baseline.current = theirs;
      sent.current = theirs;
      setText(theirs);
      setReady(true);
      return;
    }
    if (inFlight.current || dirty()) return;
    if (sent.current && sent.current.title === theirs.title && sent.current.body === theirs.body) return;
    baseline.current = theirs;
    sent.current = theirs;
    setText(theirs);
  }, [server]);

  const save = useCallback(async (): Promise<void> => {
    clearTimeout(timer.current);
    if (stopped.current) return;
    if (inFlight.current) {
      again.current = true;
      return;
    }
    if (!readyRef.current || !dirty()) return;
    const { title, body } = textRef.current;
    const id = idRef.current;
    // Nothing is created for a note left empty.
    if (id === null && !title.trim() && !body.trim()) return;
    inFlight.current = true;
    if (mounted.current) setState('saving');
    try {
      let n: PmNote;
      if (id) {
        n = await api.patchPmNote(id, { title: titleFor(title, body), body });
      } else {
        const o = newRef.current;
        n = await api.createPmNote({
          title: titleFor(title, body),
          body,
          ...(o.kind === 'user' ? { kind: 'user' as const } : {}),
          ...(o.pinned ? { pinned: true } : {}),
          ...(o.project ? { project: o.project } : {}),
        });
        idRef.current = n.id;
      }
      baseline.current = { title, body };
      sent.current = { title: n.title, body: n.body };
      qc.setQueryData(['pm-notes', 'one', n.id], n);
      if (mounted.current) {
        if (!id) {
          setNoteId(n.id);
          newRef.current.onCreated(n.id);
        }
        setState(dirty() ? 'pending' : 'saved');
      }
      void refresh();
    } catch (e) {
      if (mounted.current) setState('error');
      toast.error(`The note could not be saved: ${actionError(e)}`);
    } finally {
      inFlight.current = false;
      if (again.current) {
        again.current = false;
        void save();
      }
    }
    // The refs carry everything that changes.
  }, []);

  // Typing stopped: save.
  useEffect(() => {
    if (!ready || !dirty()) return;
    timer.current = setTimeout(() => void save(), AUTOSAVE_MS);
    return () => clearTimeout(timer.current);
  }, [text, ready, save]);

  // Leaving the note (another note, another view) saves what is unsaved.
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      if (dirty()) void save();
    };
  }, [save]);

  const edit = useCallback((patch: Partial<Text>) => {
    setText((t) => ({ ...t, ...patch }));
    setState((s) => (s === 'saving' || s === 'error' ? s : 'pending'));
  }, []);

  /** Stops saving (the note is being deleted). */
  const stop = useCallback(() => {
    stopped.current = true;
    clearTimeout(timer.current);
  }, []);

  return {
    /** The note's id; null until a new note's first save. */
    noteId,
    /** The server's copy: undefined while loading (or not created), null once it is gone. */
    server,
    loadError: one.isError ? one.error : null,
    ready,
    title: text.title,
    body: text.body,
    setTitle: (title: string) => edit({ title }),
    setBody: (body: string) => edit({ body }),
    state,
    flush: save,
    stop,
  };
}
