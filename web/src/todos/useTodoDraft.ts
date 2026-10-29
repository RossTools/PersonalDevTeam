import { useCallback, useEffect, useRef, useState } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import { toast } from '@ui';
import { api } from '../api/client';
import type { Todo } from '../api/gen/Todo';
import { actionError } from '../common/useAction';
import { AUTOSAVE_MS, type SaveState } from '../notes/useNoteDraft';
import { TODOS_KEY } from './useTodos';

interface Text {
  title: string;
  body: string;
}

/**
 * An open todo's title and body as edited here (PDT-227), saved the way notes are: AUTOSAVE_MS after typing stops, on
 * `flush` (blur, Esc, Back) and when the view goes away. The text comes from the list once; after that a polled copy
 * only replaces it when it changed elsewhere and nothing here is unsaved. An empty title isn't saved (a todo needs
 * one): the last one stays until a title is typed.
 */
export function useTodoDraft(todo: Todo | undefined) {
  const qc = useQueryClient();
  const id = todo?.id ?? null;
  const [text, setText] = useState<Text>(() => ({ title: todo?.title ?? '', body: todo?.body ?? '' }));
  const [state, setState] = useState<SaveState>('saved');
  const textRef = useRef(text);
  textRef.current = text;
  // What the server has (as edited here), and what it answered with.
  const baseline = useRef<Text>(text);
  const sent = useRef<Text>(text);
  const inFlight = useRef(false);
  const again = useRef(false);
  const stopped = useRef(false);
  const mounted = useRef(true);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const idRef = useRef(id);
  idRef.current = id;

  const changed = (a: Text, b: Text) => a.title !== b.title || a.body !== b.body;
  const dirty = () => {
    const t = textRef.current;
    return (t.title.trim() !== '' && t.title !== baseline.current.title) || t.body !== baseline.current.body;
  };

  // The polled copy: only when it changed elsewhere and nothing here is unsaved.
  const theirTitle = todo?.title;
  const theirBody = todo?.body;
  useEffect(() => {
    if (theirTitle === undefined || theirBody === undefined) return;
    const theirs = { title: theirTitle, body: theirBody };
    if (inFlight.current || dirty() || !changed(theirs, sent.current)) return;
    baseline.current = theirs;
    sent.current = theirs;
    setText(theirs);
  }, [theirTitle, theirBody]);

  const save = useCallback(async (): Promise<void> => {
    clearTimeout(timer.current);
    const todoId = idRef.current;
    if (stopped.current || !todoId) return;
    if (inFlight.current) {
      again.current = true;
      return;
    }
    if (!dirty()) return;
    const { title, body } = textRef.current;
    const patch = {
      ...(title.trim() && title !== baseline.current.title ? { title } : {}),
      ...(body !== baseline.current.body ? { body } : {}),
    };
    inFlight.current = true;
    if (mounted.current) setState('saving');
    try {
      const saved = await api.todos.patch(todoId, patch);
      baseline.current = { title: title.trim() ? title : baseline.current.title, body };
      sent.current = { title: saved.title, body: saved.body };
      qc.setQueryData<Todo[]>(TODOS_KEY, (old) => old?.map((t) => (t.id === saved.id ? saved : t)));
      if (mounted.current) setState(dirty() ? 'pending' : 'saved');
    } catch (e) {
      if (mounted.current) setState('error');
      toast.error(`The todo could not be saved: ${actionError(e)}`);
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
    if (!dirty()) return;
    timer.current = setTimeout(() => void save(), AUTOSAVE_MS);
    return () => clearTimeout(timer.current);
  }, [text, save]);

  // Leaving the todo (Back, the panel closed) saves what is unsaved.
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

  /** Stops saving (the todo is being deleted). */
  const stop = useCallback(() => {
    stopped.current = true;
    clearTimeout(timer.current);
  }, []);

  return {
    title: text.title,
    body: text.body,
    setTitle: (title: string) => edit({ title }),
    setBody: (body: string) => edit({ body }),
    state,
    flush: save,
    stop,
  };
}
