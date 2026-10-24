import { useCallback } from 'react';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { useConfirm } from '@ui';
import { api } from '../api/client';
import type { PmSessionInfo } from '../api/gen/PmSessionInfo';
import type { RotateReason } from '../api/gen/RotateReason';
import { runAction } from '../common/useAction';

/** The PM's conversations, polled with the rest of the UI (PDT-81). */
export function usePm() {
  return useQuery({ queryKey: ['pm'], queryFn: api.pm, refetchInterval: 2000 });
}

export function useRefreshPm() {
  const qc = useQueryClient();
  return () => Promise.all([qc.invalidateQueries({ queryKey: ['pm'] }), qc.invalidateQueries({ queryKey: ['pm-notes'] })]);
}

/**
 * What the PM view shows (PDT-149): `current` (the newest open conversation on a desktop, the list of conversations on
 * a phone), `new` (the box whose first message starts a conversation), `earlier` (the ended ones) or a conversation by
 * id. The notes have a view of their own (PDT-143).
 */
export type PmSelection = 'current' | 'earlier' | 'new' | (string & {});

export const REASON_LABEL: Record<RotateReason, string> = {
  context: 'its context filled up',
  age: 'it was a day old',
  user: 'you closed it',
};

/** The conversations still open, the most recently active first (the API's order). */
export const openConversations = (sessions: PmSessionInfo[]) => sessions.filter((s) => s.open);

/** The conversations that ended, newest first. */
export const earlier = (sessions: PmSessionInfo[]) => sessions.filter((s) => !s.open);

/** A journal's first line, for list rows. */
export const firstLine = (text: string | null) => (text ?? '').split('\n').find((l) => l.trim())?.replace(/^#+\s*/, '') ?? '';

/**
 * Whether a PM conversation has a reply the user hasn't seen (PDT-145): its turn ended since they last looked, it
 * isn't running a turn now, and it isn't the one on screen (`shown`; opening it marks it seen). An ended
 * conversation's last turn is its journal, not a reply.
 */
export const isUnread = ({ session: s, open }: PmSessionInfo, shown: string | null) =>
  open && s.unseen && s.activity !== 'working' && s.id !== shown;

/** Whether any PM conversation is unread: the rail's dot. */
export const pmUnread = (sessions: PmSessionInfo[] | undefined, shown: string | null) =>
  !!sessions?.some((s) => isUnread(s, shown));

/** The conversation `selection` shows: `current` is the newest open one. */
export function shownConversation(sessions: PmSessionInfo[] | undefined, current: string | null | undefined, selection: PmSelection) {
  const id = selection === 'current' ? current : selection;
  return id ? sessions?.find((s) => s.session.id === id) : undefined;
}

/** Renaming and closing a PM conversation (closing asks first). `onClosed` runs once it is closing. */
export function usePmActions(onClosed?: (id: string) => void) {
  const refresh = useRefreshPm();
  const confirm = useConfirm();
  const rename = useCallback((id: string, title: string) => {
    void runAction(async () => {
      await api.renamePm(id, title);
      await refresh();
    });
  }, [refresh]);
  const close = useCallback((info: PmSessionInfo) => {
    void runAction(async () => {
      const { ok } = await confirm({
        title: `Close “${info.title}”?`,
        body: 'It writes its journal for later conversations, then ends and stays readable. The others stay open.',
        action: 'Close',
      });
      if (!ok) return;
      await api.closePm(info.session.id);
      await refresh();
      onClosed?.(info.session.id);
    });
  }, [confirm, refresh, onClosed]);
  return { rename, close };
}
