import { act, render, renderHook, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { AgentEvent } from '../api/gen/AgentEvent';
import type { ConversationEvent } from '../api/gen/ConversationEvent';
import { FakeSocket } from './fakeSocket';
import { MessageList, type MessageListProps } from './MessageList';
import { useConversation } from './useConversation';

// Agent messages render through Markdown: counting its renders by text counts the rows' renders (PDT-136).
const renders = vi.hoisted(() => new Map<string, number>());
vi.mock('../common/Markdown', () => ({
  Markdown: ({ children }: { children: string }) => {
    renders.set(children, (renders.get(children) ?? 0) + 1);
    return <p>{children}</p>;
  },
}));

afterEach(() => {
  FakeSocket.all = [];
  renders.clear();
});

const message = (seq: number, markdown: string): ConversationEvent => ({
  seq, pos: seq, ts: seq * 1000, itemId: `m:${seq}`, turnId: 'u1',
  event: { kind: 'agent_message', id: `m${seq}`, markdown, phase: 'final' } as AgentEvent,
});

describe('MessageList renders', () => {
  it('a streamed delta re-renders only its own block; new props from a poll re-render none', () => {
    const rows = [message(1, 'one'), message(2, 'two'), message(3, 'thr')];
    const props = (streams: MessageListProps['streams']): MessageListProps => ({
      // Callers pass new closures and arrays on every render (ConversationView does).
      rows, provisional: [], streams, turnRunning: false, pending: [], onAnswer: async () => {}, onContinue: async () => {},
    });
    const { rerender } = render(<MessageList {...props({})} />);
    expect(Object.fromEntries(renders)).toEqual({ one: 1, two: 1, thr: 1 });

    rerender(<MessageList {...props({})} />);
    expect(Object.fromEntries(renders)).toEqual({ one: 1, two: 1, thr: 1 });

    // The reducer keeps a stream's entry while it is unchanged; a delta makes a new one.
    const three = { markdown: 'three' };
    rerender(<MessageList {...props({ 'm:3': three })} />);
    expect(screen.getByText('three')).toBeInTheDocument();
    expect(Object.fromEntries(renders)).toEqual({ one: 1, two: 1, thr: 1, three: 1 });

    // A new row adds its own block; the others stay.
    rerender(<MessageList {...props({ 'm:3': three })} rows={[...rows, message(4, 'four')]} />);
    expect(Object.fromEntries(renders)).toEqual({ one: 1, two: 1, thr: 1, three: 1, four: 1 });
  });
});

describe('replay', () => {
  it('applies a 2,000-event replay in one render, within 1 s', () => {
    let renderCount = 0;
    const factory = (u: string) => new FakeSocket(u);
    const { result } = renderHook(() => {
      renderCount++;
      return useConversation('s1', factory);
    });
    const server = FakeSocket.all[0];
    const start = performance.now();
    act(() => {
      server.open();
      for (let i = 1; i <= 2000; i++) server.send({ type: 'event', event: message(i, `row ${i}`) });
      server.send({ type: 'ready' });
    });
    const took = performance.now() - start;
    expect(FakeSocket.all).toHaveLength(1);
    expect(result.current.rows).toHaveLength(2000);
    expect(result.current.rows[1999].seq).toBe(2000);
    expect(result.current.ready).toBe(true);
    // Typically under 150 ms here (one render); one reducer action and render per event took about 5 s.
    expect(took).toBeLessThan(1000);
    // Mount, the socket opening (status and the open action), and the replay: not one render per event.
    expect(renderCount).toBeLessThanOrEqual(4);

    // Live events after the replay apply one by one.
    act(() => server.send({ type: 'event', event: message(2001, 'live') }));
    expect(result.current.rows).toHaveLength(2001);
  });
});
