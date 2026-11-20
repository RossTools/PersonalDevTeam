import { describe, expect, it } from 'vitest';
import type { AgentEvent } from '../api/gen/AgentEvent';
import type { ConversationEvent } from '../api/gen/ConversationEvent';
import type { ConversationServerMessage } from '../api/gen/ConversationServerMessage';
import { emptyModel, provisionalMessages, reduce, sortedRows, type ConversationModel } from './model';

const ev = (seq: number, pos: number, event: AgentEvent, itemId?: string, turnId?: string): ConversationEvent => ({
  seq, pos, ts: 1000 * seq, event, ...(itemId ? { itemId } : {}), ...(turnId ? { turnId } : {}),
});
const run = (msgs: (ConversationServerMessage | 'open')[], m: ConversationModel = emptyModel()) =>
  msgs.reduce((acc, msg) => reduce(acc, msg === 'open' ? { type: 'open' } : { type: 'message', msg }), m);
const tool = (status: 'running' | 'done', output?: string): AgentEvent =>
  ({ kind: 'tool_call', id: 't1', tool: 'Bash', title: 'ls', input: { command: 'ls' }, status, ...(output ? { output } : {}) });

describe('conversation reducer', () => {
  it('replaces an updated item by itemId and keeps conversation order by pos', () => {
    const m = run([
      { type: 'event', event: ev(1, 1, { kind: 'user_message', text: 'hi', images: [], source: 'pdt' }) },
      { type: 'event', event: ev(2, 2, tool('running'), 't:t1') },
      { type: 'event', event: ev(3, 3, { kind: 'notice', level: 'info', text: 'n' }) },
      { type: 'event', event: ev(4, 2, tool('done', 'a\nb'), 't:t1') },
    ]);
    const rows = sortedRows(m);
    expect(rows.map((r) => r.seq)).toEqual([1, 4, 3]);
    expect(rows[1].event).toMatchObject({ status: 'done', output: 'a\nb' });
    expect(m.maxSeq).toBe(4);
  });

  it('a tail holds only the latest rows; a page of earlier ones goes in before them (PDT-630)', () => {
    const say = (n: number) => ({ kind: 'notice', level: 'info', text: `n${n}` }) as const;
    // Had rows 1–3; reconnected after missing many: the server sends only the latest, from place 5.
    let m = run([
      { type: 'event', event: ev(1, 1, say(1), 'a') },
      { type: 'event', event: ev(2, 2, say(2), 'b') },
      { type: 'event', event: ev(3, 3, say(3), 'c') },
      'open',
      { type: 'tail', from: 5, earlier: true },
      { type: 'event', event: ev(9, 5, say(5), 'e') },
      { type: 'event', event: ev(10, 6, say(6), 'f') },
      // An early row changed: not held, so not shown (it comes with its page), but the socket has seen it.
      { type: 'event', event: ev(11, 2, say(22), 'b') },
      { type: 'ready' },
    ]);
    expect(sortedRows(m).map((r) => r.itemId)).toEqual(['e', 'f']);
    expect([m.from, m.earlier, m.maxSeq]).toEqual([5, true, 11]);
    m = reduce(m, { type: 'earlier', events: [ev(4, 3, say(3), 'c'), ev(11, 2, say(22), 'b'), ev(7, 4, say(4), 'd')], more: true });
    expect(sortedRows(m).map((r) => r.itemId)).toEqual(['b', 'c', 'd', 'e', 'f']);
    expect([m.from, m.earlier, m.maxSeq]).toEqual([2, true, 11]);
    // The same in one batch (a replay).
    const batched = reduce(run([{ type: 'event', event: ev(1, 1, say(1), 'a') }]), {
      type: 'batch',
      msgs: [{ type: 'tail', from: 5, earlier: true }, { type: 'event', event: ev(9, 5, say(5), 'e') }, { type: 'event', event: ev(11, 2, say(22), 'b') }],
    });
    expect(sortedRows(batched).map((r) => r.itemId)).toEqual(['e']);
    expect(batched.maxSeq).toBe(11);
    // The first page: nothing before it.
    expect(reduce(m, { type: 'earlier', events: [ev(1, 1, say(1), 'a')], more: false }).earlier).toBe(false);
    expect(reduce(m, { type: 'earlier', events: [], more: true }).earlier).toBe(false);
  });

  it('a gap filled in moves the rows after it: each place shows its newest row once', () => {
    const notice = { kind: 'notice', level: 'info', text: 'n' } as const;
    const m = run([
      { type: 'event', event: ev(1, 1, { kind: 'user_message', text: 'hi', images: [], source: 'pdt' }, 'u:1') },
      { type: 'event', event: ev(2, 2, notice) },
      // The gap: one row in after the first; the notice, which has no item id, moves down under a new seq.
      { type: 'event', event: ev(5, 3, notice) },
      { type: 'event', event: ev(6, 2, { kind: 'reasoning', summary: 'lost' }, 'th:1') },
    ]);
    expect(sortedRows(m).map((r) => [r.pos, r.seq])).toEqual([[1, 1], [2, 6], [3, 5]]);
  });

  it('ignores an older version of an item that arrives late', () => {
    const m = run([
      { type: 'event', event: ev(4, 2, tool('done'), 't:t1') },
      { type: 'event', event: ev(2, 2, tool('running'), 't:t1') },
    ]);
    expect(sortedRows(m)[0].event).toMatchObject({ status: 'done' });
  });

  it('streams an agent message before its event exists, then the event replaces the text', () => {
    let m = run([
      { type: 'ready' },
      { type: 'delta', itemId: 'm:1', field: 'markdown', text: 'Hel' },
      { type: 'delta', itemId: 'm:1', field: 'markdown', text: 'lo' },
    ]);
    expect(provisionalMessages(m)).toEqual([{ itemId: 'm:1', markdown: 'Hello' }]);
    m = run([{ type: 'event', event: ev(5, 5, { kind: 'agent_message', id: '1', markdown: 'Hello!', phase: 'final' }, 'm:1') }], m);
    expect(provisionalMessages(m)).toEqual([]);
    expect(m.streams['m:1']).toBeUndefined();
  });

  it('a running tool call keeps its streamed output; finishing drops it', () => {
    let m = run([{ type: 'ready' }, { type: 'delta', itemId: 't:t1', field: 'output', text: 'line 1\n' }]);
    m = run([{ type: 'event', event: ev(2, 2, tool('running'), 't:t1') }], m);
    expect(m.streams['t:t1']).toEqual({ output: 'line 1\n' });
    m = run([{ type: 'event', event: ev(3, 2, tool('done', 'line 1\n'), 't:t1') }], m);
    expect(m.streams['t:t1']).toBeUndefined();
  });

  it('on reconnect, a replayed delta replaces the text so far instead of doubling it', () => {
    let m = run([{ type: 'ready' }, { type: 'delta', itemId: 'm:1', field: 'markdown', text: 'Hel' }]);
    m = run(['open', { type: 'delta', itemId: 'm:1', field: 'markdown', text: 'Hello' }, { type: 'ready' }, { type: 'delta', itemId: 'm:1', field: 'markdown', text: ' you' }], m);
    expect(m.streams['m:1'].markdown).toBe('Hello you');
  });

  it('on reconnect, drops streams the replay did not mention (they finished meanwhile) and keeps the rows', () => {
    let m = run([
      { type: 'event', event: ev(1, 1, { kind: 'user_message', text: 'hi', images: [], source: 'pdt' }) },
      { type: 'ready' },
      { type: 'delta', itemId: 'm:1', field: 'markdown', text: 'Hel' },
    ]);
    m = run(['open'], m);
    expect(m.ready).toBe(false);
    m = run([{ type: 'ready' }], m);
    expect(m.streams).toEqual({});
    expect(sortedRows(m)).toHaveLength(1);
    expect(m.ready).toBe(true);
  });

  it('shows an event delivered twice (live, then in a replay) once', () => {
    const said = ev(2, 2, { kind: 'agent_message', id: 'm', markdown: 'Hello', phase: 'final' }, 'm:1');
    const user = ev(1, 1, { kind: 'user_message', text: 'hi', images: [], source: 'pdt' }, 'u:1');
    let m = run([{ type: 'event', event: user }, { type: 'event', event: said }, { type: 'ready' }]);
    m = run(['open', { type: 'event', event: user }, { type: 'event', event: said }, { type: 'event', event: said }, { type: 'ready' }], m);
    expect(sortedRows(m).map((r) => r.itemId)).toEqual(['u:1', 'm:1']);
  });

  it('a gap filled in later puts the moved rows back in order', () => {
    // Stored: the prompt, a tool call and the reply; the message between them was lost and is filled in, moving the
    // rows after it down with new seqs.
    let m = run([
      { type: 'event', event: ev(1, 1, { kind: 'user_message', text: 'hi', images: [], source: 'pdt' }, 'u:1') },
      { type: 'event', event: ev(2, 2, tool('done'), 't:t1') },
      { type: 'event', event: ev(3, 3, { kind: 'agent_message', id: 'b', markdown: 'after', phase: 'final' }, 'm:b') },
      { type: 'ready' },
    ]);
    m = run([
      'open',
      { type: 'event', event: ev(6, 4, { kind: 'agent_message', id: 'b', markdown: 'after', phase: 'final' }, 'm:b') },
      { type: 'event', event: ev(7, 3, { kind: 'agent_message', id: 'a', markdown: 'lost', phase: 'final' }, 'm:a') },
      { type: 'ready' },
    ], m);
    expect(sortedRows(m).map((r) => r.itemId)).toEqual(['u:1', 't:t1', 'm:a', 'm:b']);
  });

  it('a message still streaming when its turn ends stops showing; a tool output keeps streaming', () => {
    const state = (turnRunning: boolean) => ({ live: true, turnRunning, permissionMode: 'ask' as const, pending: [], terminalOpen: false, backgroundTasks: [] });
    let m = run([
      { type: 'state', state: state(true) },
      { type: 'ready' },
      { type: 'delta', itemId: 'm:1', field: 'markdown', text: 'Recommended now' },
      { type: 'delta', itemId: 't:t1', field: 'output', text: 'a\n' },
    ]);
    m = run([{ type: 'state', state: state(true) }], m);
    expect(provisionalMessages(m)).toHaveLength(1);
    m = run([{ type: 'state', state: state(false) }], m);
    expect(provisionalMessages(m)).toEqual([]);
    expect(m.streams).toEqual({ 't:t1': { output: 'a\n' } });
    // Stored later (its write waited): it shows as a row.
    m = run([{ type: 'event', event: ev(9, 9, { kind: 'agent_message', id: '1', markdown: 'Recommended now', phase: 'final' }, 'm:1') }], m);
    expect(sortedRows(m).map((r) => r.itemId)).toEqual(['m:1']);
  });

  it('keeps the latest state', () => {
    const state = { live: true, turnRunning: true, permissionMode: 'ask' as const, pending: [], terminalOpen: false, backgroundTasks: [] };
    expect(run([{ type: 'state', state }]).state).toEqual(state);
  });

  it('a batch ends where the same messages one by one would, without touching the model it started from', () => {
    const start = run([
      { type: 'event', event: ev(1, 1, { kind: 'user_message', text: 'hi', images: [], source: 'pdt' }) },
      { type: 'ready' },
      { type: 'delta', itemId: 'm:1', field: 'markdown', text: 'Hel' },
      { type: 'delta', itemId: 't:t1', field: 'output', text: 'a\n' },
      'open',
    ]);
    const before = structuredClone(start);
    const msgs: ConversationServerMessage[] = [
      { type: 'event', event: ev(2, 2, tool('running'), 't:t1') },
      { type: 'delta', itemId: 't:t1', field: 'output', text: 'a\nb\n' },
      { type: 'event', event: ev(3, 3, { kind: 'agent_message', id: 'm', markdown: 'Hello', phase: 'final' }, 'm:1') },
      { type: 'event', event: ev(1, 1, { kind: 'user_message', text: 'old', images: [], source: 'pdt' }) },
      { type: 'event', event: ev(4, 2, tool('done', 'a\nb\n'), 't:t1') },
      { type: 'state', state: { live: true, turnRunning: false, permissionMode: 'ask', pending: [], terminalOpen: false, backgroundTasks: [] } },
      { type: 'ready' },
    ];
    const batched = reduce(start, { type: 'batch', msgs });
    expect(batched).toEqual(run(msgs, start));
    expect(start).toEqual(before);
  });
});
