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
  /**
   * The earliest place (`pos`) held: the socket sends only a conversation's latest events (PDT-630), and rows before
   * this come a page at a time (`earlier`). 0: from the start.
   */
  from: number;
  /** There are rows before `from` to page in. */
  earlier: boolean;
}

export type ConversationAction =
  | { type: 'open' }
  | { type: 'message'; msg: ConversationServerMessage }
  /** Many messages at once (a connection's replay): one new model, not one per message. */
  | { type: 'batch'; msgs: ConversationServerMessage[] }
  /** A page of the rows before `from`, oldest first; `more`: there are rows before it too. */
  | { type: 'earlier'; events: ConversationEvent[]; more: boolean };

export const emptyModel = (): ConversationModel => ({
  rows: {}, maxSeq: 0, streams: {}, replayed: null, state: null, ready: false, from: 0, earlier: false,
});

export const rowKey = (e: ConversationEvent) => e.itemId ?? `#${e.seq}`;

function withoutStream(streams: Streams, itemId: string, keepOutput: boolean): Streams {
  const s = streams[itemId];
  if (!s) return streams;
  const next = { ...streams };
  if (keepOutput && s.output !== undefined) next[itemId] = { output: s.output };
  else delete next[itemId];
  return next;
}

function withoutMessageStreams(streams: Streams): Streams {
  if (!Object.values(streams).some((s) => s.markdown !== undefined)) return streams;
  const next: Streams = {};
  for (const [id, s] of Object.entries(streams)) {
    if (s.output !== undefined) next[id] = { output: s.output };
  }
  return next;
}

export function reduce(m: ConversationModel, a: ConversationAction): ConversationModel {
  if (a.type === 'open') return { ...m, replayed: {}, ready: false };
  if (a.type === 'batch') return reduceBatch(m, a.msgs);
  if (a.type === 'earlier') return reduceEarlier(m, a.events, a.more);
  const msg = a.msg;
  switch (msg.type) {
    case 'tail': {
      // Only the latest rows follow: the ones held before them are dropped, and paged in again when wanted.
      const rows: Record<string, ConversationEvent> = {};
      for (const [k, r] of Object.entries(m.rows)) if (r.pos >= msg.from) rows[k] = r;
      return { ...m, rows, from: msg.from, earlier: msg.earlier };
    }
    case 'event': {
      const e = msg.event;
      // Before what is held (an early row changed): it comes with the page it is in.
      if (e.pos < m.from) return e.seq > m.maxSeq ? { ...m, maxSeq: e.seq } : m;
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
    case 'state': {
      // A turn that ended without a streamed message's event: it is not coming in this turn (it may come later, as a
      // row, once PDT stores it), so its text stops showing at the end of the conversation.
      const ended = m.state?.turnRunning === true && !msg.state.turnRunning;
      return { ...m, state: msg.state, streams: ended ? withoutMessageStreams(m.streams) : m.streams };
    }
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

/**
 * `msgs` applied in order, as `reduce` would one by one, but copying the rows once: a replay of n events is O(n), not
 * O(n²).
 */
function reduceBatch(m: ConversationModel, msgs: ConversationServerMessage[]): ConversationModel {
  let cur = m;
  // Ours to change once copied; `reduce` keeps it as is for every other message.
  let rows: Record<string, ConversationEvent> | null = null;
  for (const msg of msgs) {
    if (msg.type !== 'event') {
      cur = reduce(cur, { type: 'message', msg });
      // A tail frame replaces the rows: copy them again before changing them.
      if (msg.type === 'tail') rows = null;
      continue;
    }
    const e = msg.event;
    if (e.pos < cur.from) {
      cur = { ...cur, maxSeq: Math.max(cur.maxSeq, e.seq) };
      continue;
    }
    const key = rowKey(e);
    const old = cur.rows[key];
    if (old && old.seq >= e.seq) continue;
    if (!rows) rows = { ...cur.rows };
    rows[key] = e;
    const streams = e.itemId
      ? withoutStream(cur.streams, e.itemId, e.event.kind === 'tool_call' && e.event.status === 'running')
      : cur.streams;
    cur = { ...cur, rows, maxSeq: Math.max(cur.maxSeq, e.seq), streams };
  }
  return cur;
}

/**
 * A page of rows before `from` added (the newer version of a row kept), and `from` moved back to the page's first.
 * `maxSeq` stays: it says how far the socket got, which a page read from the store says nothing about.
 */
function reduceEarlier(m: ConversationModel, events: ConversationEvent[], more: boolean): ConversationModel {
  const rows = { ...m.rows };
  let from = m.from;
  for (const e of events) {
    const key = rowKey(e);
    const old = rows[key];
    if (!old || old.seq < e.seq) rows[key] = e;
    from = Math.min(from, e.pos);
  }
  return { ...m, rows, from, earlier: more && events.length > 0 };
}

/**
 * The rows in conversation order (`pos`). Each place holds one row: a row without an item id that moved (a gap filled
 * in before it, PDT-331) comes again under its new seq, and the newer row in a place replaces the one from before.
 */
export function sortedRows(m: ConversationModel): ConversationEvent[] {
  const all = Object.values(m.rows).sort((a, b) => a.pos - b.pos || a.seq - b.seq);
  return all.filter((r, i) => i + 1 === all.length || all[i + 1].pos !== r.pos);
}

/** Agent messages that are streaming but have no stored event yet, in the order they started. */
export function provisionalMessages(m: ConversationModel): { itemId: string; markdown: string }[] {
  const have = new Set(Object.values(m.rows).flatMap((r) => (r.itemId ? [r.itemId] : [])));
  return Object.entries(m.streams).flatMap(([itemId, s]) =>
    s.markdown !== undefined && !have.has(itemId) ? [{ itemId, markdown: s.markdown }] : [],
  );
}
