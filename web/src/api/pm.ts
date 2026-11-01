import { enc, queryString, request } from './core';
import type { NewNote } from './gen/NewNote';
import type { NoteKind } from './gen/NoteKind';
import type { NotePatch } from './gen/NotePatch';
import type { PmNote } from './gen/PmNote';
import type { PmState } from './gen/PmState';
import type { Session } from './gen/Session';

/** The project manager (PDT-81): its conversations and notes. */
export const pmApi = {
  /** The project manager (PDT-81): its conversations, the open ones first (PDT-149). */
  pm: () => request<PmState>('/api/pm'),
  /** A new PM conversation beside the open ones, `text` its first message (and, without `title`, its title). */
  startPm: (text?: string, title?: string) =>
    request<Session>('/api/pm/sessions', { method: 'POST', body: JSON.stringify({ text, title }) }),
  renamePm: (id: string, title: string) =>
    request<Session>(`/api/pm/sessions/${enc(id)}`, { method: 'PATCH', body: JSON.stringify({ title }) }),
  /** Closes a PM conversation: it writes its journal, then ends. */
  closePm: (id: string) => request<void>(`/api/pm/sessions/${enc(id)}`, { method: 'DELETE' }),
  /**
   * Resets a PM conversation (PDT-308): it writes its journal and ends, and the fresh one it returns carries on with
   * its name and settings. Answers once the journal is written (up to a few minutes).
   */
  resetPm: (id: string) => request<Session>(`/api/pm/sessions/${enc(id)}/reset`, { method: 'POST' }),
  pmNotes: (q: { q?: string; project?: string; kind?: NoteKind; limit?: number } = {}) => request<PmNote[]>(`/api/pm/notes${queryString(q)}`),
  pmNote: (id: string) => request<PmNote>(`/api/pm/notes/${enc(id)}`),
  createPmNote: (n: NewNote) => request<PmNote>('/api/pm/notes', { method: 'POST', body: JSON.stringify(n) }),
  patchPmNote: (id: string, p: NotePatch) =>
    request<PmNote>(`/api/pm/notes/${enc(id)}`, { method: 'PATCH', body: JSON.stringify(p) }),
  deletePmNote: (id: string) => request<void>(`/api/pm/notes/${enc(id)}`, { method: 'DELETE' }),
};
