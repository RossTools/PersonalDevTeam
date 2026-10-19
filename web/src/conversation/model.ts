import type { ConversationEvent } from '../api/gen/ConversationEvent';
import type { ConversationServerMessage } from '../api/gen/ConversationServerMessage';
import type { ConversationState } from '../api/gen/ConversationState';
import type { DeltaField } from '../api/gen/DeltaField';

/** Streamed text per item id, not yet (or not wholly) in a stored event. */
export type Streams = Record<string, Partial<Record<DeltaField, string>>>;

/** What the events socket has told us so far. Pure data; `reduce` is the only thing that changes it. */
export interface ConversationModel {
  /** One row per item (by `itemId`, else by seq), the latest version of each. */
  rows: Record<string, ConversationEvent>;
  /** The highest seq seen: the next connection asks for what came after it. */
  maxSeq: number;
  streams: Streams;
  /**
   * While a connection replays (before `ready`): the `<itemId>:<field>` streams it has re-sent. The first replayed
   * delta of a stream is its whole text so far, so it replaces what we had instead of extending it.
   */
  replayed: Record<string, true> | null;
  state: ConversationState | null;
  /** The current connection has finished replaying. */
  ready: boolean;
}

export type ConversationAction = { type: 'open' } | { type: 'message'; msg: ConversationServerMessage };

export const emptyModel = (): ConversationModel => ({ rows: {}, maxSeq: 0, streams: {}, replayed: null, state: null, ready: false });

export const rowKey = (e: ConversationEvent) => e.itemId ?? `#${e.seq}`;

function withoutStream(streams: Streams, itemId: string, keepOutput: boolean): Streams {
  const s = streams[itemId];
  if (!s) return streams;
  const next = { ...streams };
  if (keepOutput && s.output !== undefined) next[itemId] = { output: s.output };
  else delete next[itemId];
  return next;
}

export function reduce(m: ConversationModel, a: ConversationAction): ConversationModel {
  if (a.type === 'open') return { ...m, replayed: {}, ready: false };
  const msg = a.msg;
  switch (msg.type) {
    case 'event': {
      const e = msg.event;
      const key = rowKey(e);
      const old = m.rows[key];
      if (old && old.seq >= e.seq) return m;
      // The stored item replaces its streamed text; a running tool call keeps streaming its output.
      const streams = e.itemId
        ? withoutStream(m.streams, e.itemId, e.event.kind === 'tool_call' && e.event.status === 'running')
        : m.streams;
      return { ...m, rows: { ...m.rows, [key]: e }, maxSeq: Math.max(m.maxSeq, e.seq), streams };
    }
    case 'delta': {
      const { itemId, field, text } = msg;
      const cur = m.streams[itemId] ?? {};
      let replayed = m.replayed;
      let value: string;
      if (replayed && !replayed[`${itemId}:${field}`]) {
        replayed = { ...replayed, [`${itemId}:${field}`]: true };
        value = text;
      } else {
        value = (cur[field] ?? '') + text;
      }
      return { ...m, replayed, streams: { ...m.streams, [itemId]: { ...cur, [field]: value } } };
    }
    case 'state':
      return { ...m, state: msg.state };
    case 'ready': {
      // Streams the replay didn't mention finished while we were away (their events came in the replay).
      let streams = m.streams;
      if (m.replayed) {
        const replayed = m.replayed;
        streams = {};
        for (const [id, s] of Object.entries(m.streams)) {
          const kept: Partial<Record<DeltaField, string>> = {};
          for (const f of ['markdown', 'output'] as const) {
            if (s[f] !== undefined && replayed[`${id}:${f}`]) kept[f] = s[f];
          }
          if (Object.keys(kept).length) streams[id] = kept;
        }
      }
      return { ...m, streams, replayed: null, ready: true };
    }
  }
}

/** The rows in conversation order (`pos`; seq breaks ties). */
export function sortedRows(m: ConversationModel): ConversationEvent[] {
  return Object.values(m.rows).sort((a, b) => a.pos - b.pos || a.seq - b.seq);
}

/** Agent messages that are streaming but have no stored event yet, in the order they started. */
export function provisionalMessages(m: ConversationModel): { itemId: string; markdown: string }[] {
  const have = new Set(Object.values(m.rows).flatMap((r) => (r.itemId ? [r.itemId] : [])));
  return Object.entries(m.streams).flatMap(([itemId, s]) =>
    s.markdown !== undefined && !have.has(itemId) ? [{ itemId, markdown: s.markdown }] : [],
  );
}
