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
 * What the PM view shows: the current session, the list of earlier sessions, or an earlier session (its id). The
 * notes have a view of their own (PDT-143).
 */
export type PmSelection = 'current' | 'earlier' | (string & {});

export const REASON_LABEL: Record<RotateReason, string> = {
  context: 'its context filled up',
  age: 'it was a day old',
  user: 'you started a new one',
};

/** The sessions that ended, newest first. */
export const earlier = (sessions: PmSessionInfo[]) => sessions.filter((s) => s.rotation === 'done');

/** A journal's first line, for list rows. */
export const firstLine = (text: string | null) => (text ?? '').split('\n').find((l) => l.trim())?.replace(/^#+\s*/, '') ?? '';
