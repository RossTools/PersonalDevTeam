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
