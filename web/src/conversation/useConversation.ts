import { useEffect, useMemo, useReducer, useRef, useState } from 'react';
import type { ConversationServerMessage } from '../api/gen/ConversationServerMessage';
import { EventsSocket, eventsUrl, type EventsSocketLike, type EventsStatus } from './eventsSocket';
import { emptyModel, provisionalMessages, reduce, sortedRows } from './model';

/**
 * A conversation's events, live: replays what is stored, then follows new and updated rows, streaming deltas and
 * the conversation state. Reconnects (asking only for what it lacks) and keeps what it has meanwhile. Key the
 * caller by session id: the model is not reset when `sessionId` changes.
 */
export function useConversation(sessionId: string, factory?: (url: string) => EventsSocketLike) {
  const [model, dispatch] = useReducer(reduce, undefined, emptyModel);
  const [status, setStatus] = useState<EventsStatus>('connecting');
  const maxSeq = useRef(0);
  maxSeq.current = model.maxSeq;

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
    sock.connect();
    return () => sock.dispose();
  }, [sessionId, factory]);

  const rows = useMemo(() => sortedRows(model), [model.rows]);
  const provisional = useMemo(() => provisionalMessages(model), [model.rows, model.streams]);
  return { rows, provisional, streams: model.streams, state: model.state, ready: model.ready, status };
}

export type ConversationData = ReturnType<typeof useConversation>;
