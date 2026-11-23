import { act, fireEvent, render, renderHook, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { AgentEvent } from '../api/gen/AgentEvent';
import type { ConversationEvent } from '../api/gen/ConversationEvent';
import type { Session } from '../api/gen/Session';
import { readViewState, writeViewState } from '../common/viewState';
import { renderWithQuery, stubApi } from '../test-utils';
import { ConversationView } from './ConversationView';
import { FakeSocket } from './fakeSocket';
import { MessageList, WINDOW, type MessageListProps } from './MessageList';
import { EARLIER_PAGE, useConversation } from './useConversation';

vi.mock('../common/Markdown', () => ({
  Markdown: ({ children }: { children: string }) => <p>{children}</p>,
}));

afterEach(() => {
  FakeSocket.all = [];
});

const message = (seq: number): ConversationEvent => ({
  seq, pos: seq, ts: seq * 1000, itemId: `m:${seq}`, turnId: `u${seq}`,
  event: { kind: 'agent_message', id: `m${seq}`, markdown: `row ${seq}`, phase: 'final' } as AgentEvent,
});
const messages = (n: number) => Array.from({ length: n }, (_, i) => message(i + 1));

const props = (rows: ConversationEvent[], extra: Partial<MessageListProps> = {}): MessageListProps => ({
  rows, provisional: [], streams: {}, turnRunning: false, pending: [], onAnswer: async () => {}, ...extra,
});

/** Each shown block 100 px tall, in a 500 px window onto the conversation (the log). */
function layout() {
  const blocks = (log: Element) => Array.from(log.querySelectorAll<HTMLElement>('[data-block]'));
  const isLog = (el: Element) => el.getAttribute('role') === 'log';
  vi.spyOn(Element.prototype, 'scrollHeight', 'get').mockImplementation(function (this: Element) {
    return isLog(this) ? blocks(this).length * 100 : 0;
  });
  vi.spyOn(Element.prototype, 'clientHeight', 'get').mockImplementation(function (this: Element) {
    return isLog(this) ? 500 : 0;
  });
  vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
    const log = this.closest('[role="log"]');
    const i = log ? blocks(log).indexOf(this) : -1;
    return new DOMRect(0, i < 0 || !log ? 0 : i * 100 - log.scrollTop, 100, 100);
  });
}

describe('a long conversation opens on its latest messages', () => {
  it('shows the last blocks at first, and earlier ones a window at a time, keeping the reader in place', async () => {
    layout();
    render(<MessageList {...props(messages(120))} />);
    const log = screen.getByRole('log');
    expect(log.scrollTop).toBe(WINDOW * 100);
    expect(screen.getByText('row 120')).toBeInTheDocument();
    expect(screen.getByText(`row ${120 - WINDOW + 1}`)).toBeInTheDocument();
    expect(screen.queryByText(`row ${120 - WINDOW}`)).not.toBeInTheDocument();

    // Scrolled up a little: "row 82" is at the top of the screen.
    log.scrollTop = 1100;
    fireEvent.scroll(log);
    expect(screen.getByRole('button', { name: 'Jump to latest' })).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Show earlier messages' }));
    expect(screen.getByText(`row ${120 - 2 * WINDOW + 1}`)).toBeInTheDocument();
    // Fifty blocks came in above: the screen moved down by as much, so it shows what it showed.
    expect(log.scrollTop).toBe(1100 + WINDOW * 100);

    // Near the top: the rest come in on their own, and there is nothing earlier left to show.
    log.scrollTop = 200;
    fireEvent.scroll(log);
    expect(screen.getByText('row 1')).toBeInTheDocument();
    expect(log.scrollTop).toBe(200 + 20 * 100);
    expect(screen.queryByRole('button', { name: 'Show earlier messages' })).not.toBeInTheDocument();
  });

  it('once every block held is shown, reads the page before them from the server (PDT-630)', async () => {
    layout();
    const onLoadEarlier = vi.fn();
    const { rerender } = render(<MessageList {...props(messages(30), { hasEarlier: true, onLoadEarlier })} />);
    await userEvent.click(screen.getByRole('button', { name: 'Show earlier messages' }));
    expect(onLoadEarlier).toHaveBeenCalledTimes(1);
    rerender(<MessageList {...props(messages(30), { hasEarlier: false, onLoadEarlier })} />);
    expect(screen.queryByRole('button', { name: 'Show earlier messages' })).not.toBeInTheDocument();
  });

  it('a tool call sent with a preview of its output reads the whole when asked', async () => {
    const call: ConversationEvent = {
      seq: 1, pos: 1, ts: 1, itemId: 't:1', outputBytes: 40_000,
      event: { kind: 'tool_call', id: '1', tool: 'Bash', title: 'cat big', input: { command: 'cat big' }, status: 'done', output: 'the start' } as AgentEvent,
    };
    const fullOutput = vi.fn(async () => 'the start and the rest');
    render(<MessageList {...props([call], { fullOutput })} />);
    await userEvent.click(screen.getByRole('button', { name: /cat big/ }));
    expect(screen.getByText('the start')).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: /Show all output/ }));
    expect(fullOutput).toHaveBeenCalledWith('t:1');
    expect(await screen.findByText('the start and the rest')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /Show all output/ })).not.toBeInTheDocument();
  });

  it('keeps the earliest block shown as new ones arrive, and follows the latest while at the bottom', () => {
    layout();
    const { rerender } = render(<MessageList {...props(messages(60))} />);
    expect(screen.queryByText('row 10')).not.toBeInTheDocument();
    expect(screen.getByText('row 11')).toBeInTheDocument();
    const log = screen.getByRole('log');
    rerender(<MessageList {...props(messages(61))} />);
    expect(screen.getByText('row 11')).toBeInTheDocument();
    expect(screen.getByText('row 61')).toBeInTheDocument();
    expect(log.scrollTop).toBe(log.scrollHeight);
  });

  it('a short conversation shows everything, without a way to show more', () => {
    render(<MessageList {...props(messages(3))} />);
    expect(screen.getByText('row 1')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Show earlier messages' })).not.toBeInTheDocument();
  });

  it('opens where it was left, by its distance from the bottom', () => {
    layout();
    const { unmount } = render(<MessageList {...props(messages(120))} remember="conversation:s1" />);
    const log = screen.getByRole('log');
    log.scrollTop = 4000;
    fireEvent.scroll(log);
    expect(readViewState('app|conversation:s1')).toEqual({ fromBottom: 500 });
    unmount();

    // Left 6,000 px from the bottom: more than the first window holds, so the window before it comes in too.
    writeViewState('app|conversation:s1', { fromBottom: 6000 });
    render(<MessageList {...props(messages(120))} remember="conversation:s1" />);
    expect(screen.getByText('row 21')).toBeInTheDocument();
    expect(screen.queryByText('row 20')).not.toBeInTheDocument();
    expect(screen.getByRole('log').scrollTop).toBe(100 * 100 - 500 - 6000);
    expect(screen.getByRole('button', { name: 'Jump to latest' })).toBeInTheDocument();
  });
});

describe('opening a conversation', () => {
  const session = {
    id: 's1', projectId: 'p', worktreeId: null, kind: 'agent_tui', agent: 'claude', bypassPermissions: false, title: 'PDT', cwd: '/r',
    status: 'running', exitCode: null, createdAt: 1, endedAt: null, externalSessionId: null, freshStartedAt: null, activity: 'idle',
    attentionAt: null, lastViewedAt: null, unseen: false, attemptId: null, ticket: null, mode: 'conversation', model: null, permissionMode: 'ask', persona: null, background: [], instance: null,
  } as Session;

  it('shows that it is loading until the stored messages are in, never a blank pane', () => {
    stubApi({ 'GET /api/agents/claude/capabilities': { conversation: true, permissionModes: [], models: [], commands: [] } });
    renderWithQuery(<ConversationView session={session} socketFactory={(u) => new FakeSocket(u)} />);
    expect(screen.getByRole('status', { name: 'Loading the conversation' })).toBeInTheDocument();
    const server = FakeSocket.all[0];
    act(() => {
      server.open();
      server.send({ type: 'event', event: message(1) });
      server.send({ type: 'ready' });
    });
    expect(screen.queryByRole('status', { name: 'Loading the conversation' })).not.toBeInTheDocument();
    expect(screen.getByText('row 1')).toBeInTheDocument();
  });

  it('a replay that takes too long shows what came so far, with Retry; one that stalls is made again (PDT-630)', () => {
    vi.useFakeTimers();
    try {
      stubApi({ 'GET /api/agents/claude/capabilities': { conversation: true, permissionModes: [], models: [], commands: [] } });
      renderWithQuery(<ConversationView session={session} socketFactory={(u) => new FakeSocket(u)} />);
      const server = FakeSocket.all[0];
      act(() => {
        server.open();
        for (let i = 1; i <= 3; i++) {
          vi.advanceTimersByTime(3000);
          server.send({ type: 'event', event: message(i) });
        }
        vi.advanceTimersByTime(1000);
      });
      // Ten seconds and no `ready`: no skeleton, what came shows, and the rest applies as it comes.
      expect(screen.queryByRole('status', { name: 'Loading the conversation' })).not.toBeInTheDocument();
      expect(screen.getByText('row 3')).toBeInTheDocument();
      expect(screen.getByText(/Still loading/)).toBeInTheDocument();
      act(() => server.send({ type: 'event', event: message(4) }));
      expect(screen.getByText('row 4')).toBeInTheDocument();
      // Retry: a new connection at once, asking for what came after what it has.
      act(() => {
        fireEvent.click(screen.getByRole('button', { name: 'Retry' }));
        vi.advanceTimersByTime(0);
      });
      const again = FakeSocket.all[1];
      expect(again.url).toMatch(/after=4$/);
      // It stalls before opening: dropped and made again, with what came kept on screen.
      act(() => vi.advanceTimersByTime(8000 + 1000));
      expect(FakeSocket.all).toHaveLength(3);
      expect(screen.getByText('row 4')).toBeInTheDocument();
      act(() => {
        FakeSocket.all[2].open();
        FakeSocket.all[2].send({ type: 'ready' });
      });
      expect(screen.queryByText(/Still loading/)).not.toBeInTheDocument();
    } finally {
      vi.useRealTimers();
    }
  });

  it('opens on the latest events and reads earlier ones a page at a time (PDT-630)', async () => {
    const calls = stubApi({ 'GET /api/sessions/s1/events': (c: { path: string }) => {
      const before = Number(new URL(c.path, 'http://x').searchParams.get('before'));
      return messages(before - 1).slice(-EARLIER_PAGE);
    } });
    const factory = (u: string) => new FakeSocket(u);
    const h = renderHook(() => useConversation('s1', factory));
    act(() => {
      FakeSocket.all[0].open();
      FakeSocket.all[0].send({ type: 'tail', from: 401, earlier: true });
      for (const e of messages(450).slice(400)) FakeSocket.all[0].send({ type: 'event', event: e });
      FakeSocket.all[0].send({ type: 'ready' });
    });
    expect(h.result.current.rows).toHaveLength(50);
    expect(h.result.current.hasEarlier).toBe(true);
    act(() => h.result.current.loadEarlier());
    // Asked once while a page is being read.
    act(() => h.result.current.loadEarlier());
    await waitFor(() => expect(h.result.current.rows).toHaveLength(250));
    expect(calls.map((c) => c.path)).toEqual([`/api/sessions/s1/events?before=401&limit=${EARLIER_PAGE}`]);
    expect(h.result.current.rows[0].seq).toBe(201);
    act(() => h.result.current.loadEarlier());
    await waitFor(() => expect(h.result.current.rows).toHaveLength(450));
    expect(h.result.current.hasEarlier).toBe(true);
    act(() => h.result.current.loadEarlier());
    await waitFor(() => expect(h.result.current.hasEarlier).toBe(false));
    expect(h.result.current.rows).toHaveLength(450);
  });

  it('one opened lately shows what it had at once, and asks only for what came after', () => {
    const factory = (u: string) => new FakeSocket(u);
    const first = renderHook(() => useConversation('s1', factory));
    act(() => {
      FakeSocket.all[0].open();
      for (const e of messages(3)) FakeSocket.all[0].send({ type: 'event', event: e });
      FakeSocket.all[0].send({ type: 'ready' });
    });
    first.unmount();
    // Another conversation in between.
    renderHook(() => useConversation('s2', factory)).unmount();

    const back = renderHook(() => useConversation('s1', factory));
    expect(back.result.current.rows.map((r) => r.seq)).toEqual([1, 2, 3]);
    const server = FakeSocket.all[FakeSocket.all.length - 1];
    expect(server.url).toMatch(/\/s1\/events\?after=3$/);
    act(() => {
      server.open();
      server.send({ type: 'event', event: message(4) });
      server.send({ type: 'ready' });
    });
    expect(back.result.current.rows.map((r) => r.seq)).toEqual([1, 2, 3, 4]);
  });

  it('keeps only the last few conversations', () => {
    const factory = (u: string) => new FakeSocket(u);
    for (const id of ['a', 'b', 'c', 'd', 'e', 'f']) {
      const h = renderHook(() => useConversation(id, factory));
      const server = FakeSocket.all[FakeSocket.all.length - 1];
      act(() => {
        server.open();
        server.send({ type: 'event', event: message(1) });
        server.send({ type: 'ready' });
      });
      h.unmount();
    }
    expect(renderHook(() => useConversation('a', factory)).result.current.rows).toEqual([]);
    expect(renderHook(() => useConversation('f', factory)).result.current.rows).toHaveLength(1);
  });
});
