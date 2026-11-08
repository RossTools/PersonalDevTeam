import { useState } from 'react';
import { keepPreviousData, useQuery } from '@tanstack/react-query';
import { api, ApiError } from '../api/client';
import { ALL, type Whose } from './model';
import { POLL } from '../common/poll';
import { pmKeys } from '../pm/queries';

/** The list's choices, kept by the shell so the list and a new note agree: whose, the scope and the search. */
export interface NotesFilters {
  whose: Whose;
  setWhose: (w: Whose) => void;
  scope: string;
  setScope: (s: string) => void;
  q: string;
  setQ: (q: string) => void;
}

export function useNotesFilters(): NotesFilters {
  const [whose, setWhose] = useState<Whose>('user');
  const [scope, setScope] = useState(ALL);
  const [q, setQ] = useState('');
  return { whose, setWhose, scope, setScope, q, setQ };
}

/**
 * The `limit` newest notes of one side (full-text search on the server), polled with the rest of the UI; the last
 * answer shows while the next loads.
 */
export function useNotesList(whose: Whose, q: string, limit: number) {
  return useQuery({
    queryKey: pmKeys.notesList(whose, q, limit),
    queryFn: () => api.pmNotes({ q: q.trim() || undefined, kind: whose, limit }),
    refetchInterval: POLL.live,
    placeholderData: keepPreviousData,
  });
}

/** One note (null once it is gone), polled. Disabled for a note not created yet. */
export function useNote(id: string | null) {
  return useQuery({
    queryKey: pmKeys.note(id),
    queryFn: () => api.pmNote(id!).catch((e: unknown) => {
      if (e instanceof ApiError && e.status === 404) return null;
      throw e;
    }),
    enabled: id !== null,
    refetchInterval: POLL.live,
  });
}
