import { useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { api, ApiError } from '../api/client';
import { ALL, type Whose } from './model';

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

/** The notes of one side (full-text search on the server), polled with the rest of the UI. */
export function useNotesList(whose: Whose, q: string) {
  return useQuery({
    queryKey: ['pm-notes', whose, q],
    queryFn: () => api.pmNotes({ q: q.trim() || undefined, kind: whose }),
    refetchInterval: 2000,
  });
}

/** One note (null once it is gone), polled. Disabled for a note not created yet. */
export function useNote(id: string | null) {
  return useQuery({
    queryKey: ['pm-notes', 'one', id],
    queryFn: () => api.pmNote(id!).catch((e: unknown) => {
      if (e instanceof ApiError && e.status === 404) return null;
      throw e;
    }),
    enabled: id !== null,
    refetchInterval: 2000,
  });
}

export function useProjects() {
  return useQuery({ queryKey: ['projects'], queryFn: api.listProjects });
}
