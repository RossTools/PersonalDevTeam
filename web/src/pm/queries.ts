// The PM: its conversations and its notes, each one query shared by every screen that shows it.
import type { Whose } from '../notes/model';

export const pmKeys = {
  /** The PM's conversations (`GET /api/pm`). */
  pm: ['pm'] as const,
  /** Every query of notes: a write to one refreshes the lists and the note alike. */
  notes: ['pm-notes'] as const,
  notesList: (whose: Whose, q: string, limit: number) => ['pm-notes', 'list', whose, q, limit] as const,
  note: (id: string | null) => ['pm-notes', 'one', id] as const,
};
