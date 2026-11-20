import { useCallback, useEffect, useMemo, useReducer, useRef, useState } from 'react';
import { api } from '../api/client';
import type { ConversationServerMessage } from '../api/gen/ConversationServerMessage';
import { EventsSocket, eventsUrl, type EventsSocketLike, type EventsStatus } from './eventsSocket';
import { emptyModel, provisionalMessages, reduce, sortedRows, type ConversationModel } from './model';

/** How many earlier rows each step back reads. */
export const EARLIER_PAGE = 200;
/** How many conversations' models are kept for the page once left. */
const KEEP = 5;
/** The last conversations shown, least recent first. */
const kept = new Map<string, ConversationModel>();

function keep(sessionId: string, m: ConversationModel) {
  kept.delete(sessionId);
  kept.set(sessionId, m);
  for (const id of kept.keys()) {
    if (kept.size <= KEEP) break;
    kept.delete(id);
  }
}

/** Tests: no conversation is kept from one test to the next. */
export function resetConversations() {
  kept.clear();
}

/**
 * A conversation's events, live: replays what is stored (its latest events; `loadEarlier` pages back), then follows
 * new and updated rows, streaming deltas and the conversation state. Reconnects (asking only for what it lacks) and
 * keeps what it has meanwhile. Key the caller by session id: the model is not reset when `sessionId` changes. One of
 * the last few conversations shown starts from what it had (PDT-619), so going back to it shows it at once and asks
 * only for what came after. A replay that is slow shows what came so far and says so (`slow`, with `retry`).
 */
export function useConversation(sessionId: string, factory?: (url: string) => EventsSocketLike) {
  const [model, dispatch] = useReducer(reduce, sessionId, (id) => kept.get(id) ?? emptyModel());
  const [status, setStatus] = useState<EventsStatus>('connecting');
  const [slow, setSlow] = useState(false);
  const [loadingEarlier, setLoadingEarlier] = useState(false);
  const socket = useRef<EventsSocket | null>(null);
  const maxSeq = useRef(0);
  maxSeq.current = model.maxSeq;
  const from = useRef(model.from);
  from.current = model.from;
  const loading = useRef(false);
  useEffect(() => {
    if (model.ready) setSlow(false);
  }, [model.ready]);
  useEffect(() => {
    if (model.ready) keep(sessionId, model);
  }, [sessionId, model]);

  useEffect(() => {
    // A connection's replay (everything up to `ready`) is held and applied in one action: thousands of stored events
    // are one render, not one each (PDT-136). Live messages after it apply as they come.
    let replay: ConversationServerMessage[] | null = null;
    const flush = () => {
      if (replay?.length) dispatch({ type: 'batch', msgs: replay });
      replay = null;
    };
    const sock = new EventsSocket(
      (after) => eventsUrl(sessionId, after),
      () => maxSeq.current,
      {
        onOpen: () => {
          dispatch({ type: 'open' });
          replay = [];
        },
        // Slow: what came so far shows, and the rest applies as it comes.
        onSlow: () => {
          flush();
          setSlow(true);
        },
        onMessage: (msg) => {
          if (!replay) {
            dispatch({ type: 'message', msg });
            return;
          }
          replay.push(msg);
          if (msg.type === 'ready') flush();
        },
        onStatus: (st) => {
          // A connection that dropped mid-replay: keep what it sent (the next one asks only for what came after).
          if (st === 'reconnecting') flush();
          setStatus(st);
        },
      },
      factory,
    );
    socket.current = sock;
    sock.connect();
    return () => {
      sock.dispose();
      socket.current = null;
    };
  }, [sessionId, factory]);

  const retry = useCallback(() => socket.current?.retry(), []);
  /** Reads the page of rows before the earliest held. */
  const loadEarlier = useCallback(() => {
    if (loading.current || from.current <= 0) return;
    loading.current = true;
    setLoadingEarlier(true);
    const before = from.current;
    api.sessionEventsBefore(sessionId, before, EARLIER_PAGE)
      .then((events) => dispatch({ type: 'earlier', events, more: events.length === EARLIER_PAGE }))
      .catch(() => {})
      .finally(() => {
        loading.current = false;
        setLoadingEarlier(false);
      });
  }, [sessionId]);

  const rows = useMemo(() => sortedRows(model), [model.rows]);
  const provisional = useMemo(() => provisionalMessages(model), [model.rows, model.streams]);
  return {
    rows, provisional, streams: model.streams, state: model.state, ready: model.ready, status, slow, retry,
    hasEarlier: model.earlier, loadingEarlier, loadEarlier,
  };
}

export type ConversationData = ReturnType<typeof useConversation>;
