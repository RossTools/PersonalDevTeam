import { useQuery, useQueryClient } from '@tanstack/react-query';
import { api } from '../api/client';
import type { PmSessionInfo } from '../api/gen/PmSessionInfo';
import type { RotateReason } from '../api/gen/RotateReason';

/** The PM's sessions, polled with the rest of the UI (PDT-81). */
export function usePm() {
  return useQuery({ queryKey: ['pm'], queryFn: api.pm, refetchInterval: 2000 });
}

export function useRefreshPm() {
  const qc = useQueryClient();
  return () => Promise.all([qc.invalidateQueries({ queryKey: ['pm'] }), qc.invalidateQueries({ queryKey: ['pm-notes'] })]);
}

/**
 * What the PM view shows: the current session, the notes, one note (`note:<id>`), the list of earlier sessions, or an
 * earlier session (its id).
 */
export type PmSelection = 'current' | 'notes' | 'earlier' | `note:${string}` | (string & {});

/** The note a selection shows, if it is one. */
export const noteOf = (s: PmSelection): string | null => (s.startsWith('note:') ? s.slice(5) : null);

export const REASON_LABEL: Record<RotateReason, string> = {
  context: 'its context filled up',
  age: 'it was a day old',
  user: 'you started a new one',
};

/** The sessions that ended, newest first. */
export const earlier = (sessions: PmSessionInfo[]) => sessions.filter((s) => s.rotation === 'done');

/** A journal's first line, for list rows. */
export const firstLine = (text: string | null) => (text ?? '').split('\n').find((l) => l.trim())?.replace(/^#+\s*/, '') ?? '';

/**
 * Whether a PM session has a reply the user hasn't seen (PDT-145): its turn ended since they last looked, it isn't
 * running a turn now, and it isn't the one on screen (`shown`; opening it marks it seen). An ended session's last turn
 * is its journal, not a reply.
 */
export const pmUnread = (sessions: PmSessionInfo[] | undefined, shown: string | null) =>
  !!sessions?.some(({ session: s, rotation }) => rotation !== 'done' && s.unseen && s.activity !== 'working' && s.id !== shown);
