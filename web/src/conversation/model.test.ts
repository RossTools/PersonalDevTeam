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

  it('keeps the latest state', () => {
    const state = { live: true, turnRunning: true, permissionMode: 'ask' as const, pending: [] };
    expect(run([{ type: 'state', state }]).state).toEqual(state);
  });
});
