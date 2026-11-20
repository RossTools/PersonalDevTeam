import { afterEach, describe, expect, it, vi } from 'vitest';
import { EventsSocket, eventsUrl } from './eventsSocket';
import { FakeSocket } from './fakeSocket';

afterEach(() => {
  FakeSocket.all = [];
  vi.useRealTimers();
});

describe('EventsSocket', () => {
  it('builds ws and wss urls', () => {
    expect(eventsUrl('a b', 3, { protocol: 'http:', host: 'h:1' })).toBe('ws://h:1/api/sessions/a%20b/events?after=3');
    expect(eventsUrl('s', 0, { protocol: 'https:', host: 'h' })).toBe('wss://h/api/sessions/s/events?after=0');
  });

  it('reconnects with backoff, asking for what came after the highest seq', () => {
    vi.useFakeTimers();
    let after = 0;
    const onMessage = vi.fn();
    const onOpen = vi.fn();
    const s = new EventsSocket((a) => `ws://x/events?after=${a}`, () => after, { onOpen, onMessage }, (u) => new FakeSocket(u));
    s.connect();
    FakeSocket.all[0].open();
    expect(onOpen).toHaveBeenCalledTimes(1);
    FakeSocket.all[0].send({ type: 'ready' });
    expect(onMessage).toHaveBeenCalledWith({ type: 'ready' });
    after = 7;
    FakeSocket.all[0].close();
    vi.advanceTimersByTime(500);
    expect(FakeSocket.all[1].url).toBe('ws://x/events?after=7');
    s.dispose();
    FakeSocket.all[1].close();
    vi.advanceTimersByTime(10_000);
    expect(FakeSocket.all).toHaveLength(2);
  });

  it('drops a connection that stops sending before ready and makes a new one; says when a replay is slow', () => {
    vi.useFakeTimers();
    let after = 0;
    const onMessage = vi.fn();
    const onSlow = vi.fn();
    const statuses: string[] = [];
    const s = new EventsSocket((a) => `ws://x/events?after=${a}`, () => after, {
      onOpen: () => {}, onMessage, onSlow, onStatus: (st) => statuses.push(st),
    }, (u) => new FakeSocket(u), { stallMs: 3000, readyMs: 5000 });
    s.connect();
    // Never opens: dropped once it has been quiet for stallMs.
    vi.advanceTimersByTime(3000);
    expect(statuses).toEqual(['connecting', 'reconnecting']);
    vi.advanceTimersByTime(500);
    const second = FakeSocket.all[1];
    second.open();
    // Replaying, frame by frame: still making progress, so kept, but slow once readyMs pass.
    for (let i = 1; i <= 4; i++) {
      vi.advanceTimersByTime(2000);
      second.send({ type: 'event', event: { seq: i } });
    }
    expect(onSlow).toHaveBeenCalledTimes(1);
    expect(FakeSocket.all).toHaveLength(2);
    after = 4;
    // Then nothing: dropped, and the next asks for what came after what it got. What the dropped one sends is ignored.
    vi.advanceTimersByTime(3000);
    second.send({ type: 'event', event: { seq: 5 } });
    expect(onMessage).toHaveBeenCalledTimes(4);
    vi.advanceTimersByTime(1000);
    const third = FakeSocket.all[2];
    expect(third.url).toBe('ws://x/events?after=4');
    third.open();
    third.send({ type: 'ready' });
    // Ready: quiet is fine from here on.
    vi.advanceTimersByTime(60_000);
    expect(FakeSocket.all).toHaveLength(3);
    expect(onSlow).toHaveBeenCalledTimes(1);
    s.dispose();
  });

  it('Retry makes a new connection at once', () => {
    vi.useFakeTimers();
    const s = new EventsSocket((a) => `ws://x/events?after=${a}`, () => 0, { onOpen: () => {}, onMessage: () => {} }, (u) => new FakeSocket(u));
    s.connect();
    FakeSocket.all[0].open();
    s.retry();
    vi.advanceTimersByTime(0);
    expect(FakeSocket.all).toHaveLength(2);
    expect(FakeSocket.all[0].readyState).toBe(3);
    s.dispose();
  });
});
