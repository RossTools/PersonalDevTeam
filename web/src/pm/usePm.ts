import { useCallback, useEffect, useRef } from 'react';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { useConfirm } from '@ui';
import { api } from '../api/client';
import type { PmSessionInfo } from '../api/gen/PmSessionInfo';
import type { RotateReason } from '../api/gen/RotateReason';
import { runAction } from '../common/useAction';
import { pluginsKey } from '../plugins/model';

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

/**
 * Why a conversation ended, after "because". Only the user's Close and Reset end one now (PDT-167, PDT-308); the
 * others are older.
 */
export const REASON_LABEL: Record<RotateReason, string> = {
  context: 'its context filled up',
  age: 'it was a day old',
  user: 'you closed it',
  reset: 'you reset it',
};

/** The conversations still open, the most recently active first (the API's order). */
export const openConversations = (sessions: PmSessionInfo[]) => sessions.filter((s) => s.open);

/**
 * The conversations that ended, newest first. A reset one is left out when a conversation carries on from it (PDT-308):
 * that one links to it. One whose reset never finished is listed like a closed one.
 */
export const earlier = (sessions: PmSessionInfo[]) => {
  const carriedOn = new Set(sessions.map((s) => s.resetFrom).filter(Boolean));
  return sessions.filter((s) => !s.open && !carriedOn.has(s.session.id));
};

/**
 * Whether a conversation's reset is under way (PDT-308): it is writing its journal, or has just ended and its fresh one
 * is starting (still open). An ended one that was reset is not.
 */
export const resetting = (info: PmSessionInfo) => info.open && info.rotation !== 'none' && info.reason === 'reset';

/** A journal's first line, for list rows. */
/** Where a conversation is held besides PDT, for its list row and header (PDT-304: a chat plugin's DMs): `via` is the
 * plugin's id, named as Settings → Plugins names it. */
export function useViaLabel() {
  const plugins = useQuery({ queryKey: pluginsKey, queryFn: api.listIntegrations, staleTime: 60_000 });
  return (info: PmSessionInfo) => (info.via ? `via ${plugins.data?.find((p) => p.id === info.via)?.label ?? info.via}` : null);
}
export const firstLine = (text: string | null) => (text ?? '').split('\n').find((l) => l.trim())?.replace(/^#+\s*/, '') ?? '';

/**
 * Whether a PM conversation has a reply the user hasn't seen (PDT-145): its turn ended since they last looked, it
 * isn't running a turn now, and it isn't the one on screen (`shown`; opening it marks it seen). An ending or ended
 * conversation's last turn is its journal, not a reply.
 */
export const isUnread = ({ session: s, open, rotation }: PmSessionInfo, shown: string | null) =>
  open && rotation === 'none' && s.unseen && s.activity !== 'working' && s.id !== shown;

/** Whether any PM conversation is unread: the rail's dot. */
export const pmUnread = (sessions: PmSessionInfo[] | undefined, shown: string | null) =>
  !!sessions?.some((s) => isUnread(s, shown));

/** The conversation `selection` shows: `current` is the newest open one. */
export function shownConversation(sessions: PmSessionInfo[] | undefined, current: string | null | undefined, selection: PmSelection) {
  const id = selection === 'current' ? current : selection;
  return id ? sessions?.find((s) => s.session.id === id) : undefined;
}

/**
 * The conversation on show now (null once the view is gone), for an action that answers later: a reset that takes
 * minutes switches the screen to the fresh conversation only if the old one is still on show (PDT-308).
 */
export function useShownRef(shown: string | null) {
  const ref = useRef<string | null>(shown);
  useEffect(() => { ref.current = shown; }, [shown]);
  useEffect(() => () => { ref.current = null; }, []);
  return ref;
}

/**
 * Renaming, closing and resetting a PM conversation (closing and resetting ask first). `onClosed` runs once it is
 * closing, `onReset` with the fresh conversation once a reset is done (PDT-308).
 */
export function usePmActions(onClosed?: (id: string) => void, onReset?: (id: string, fresh: string) => void) {
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
  const reset = useCallback((info: PmSessionInfo) => {
    void runAction(async () => {
      const { ok } = await confirm({
        title: `Reset “${info.title}”?`,
        body: 'Starts this conversation fresh. Its journal is kept and the new one starts from it.',
        action: 'Reset',
      });
      if (!ok) return;
      // It answers once the journal is written: the conversation shows it resetting meanwhile.
      const fresh = await api.resetPm(info.session.id);
      await refresh();
      onReset?.(info.session.id, fresh.id);
    });
  }, [confirm, refresh, onReset]);
  return { rename, close, reset };
}
