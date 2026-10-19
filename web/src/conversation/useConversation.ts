import { useEffect, useMemo, useReducer, useRef, useState } from 'react';
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
    const sock = new EventsSocket(
      (after) => eventsUrl(sessionId, after),
      () => maxSeq.current,
      {
        onOpen: () => dispatch({ type: 'open' }),
        onMessage: (msg) => dispatch({ type: 'message', msg }),
        onStatus: setStatus,
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
