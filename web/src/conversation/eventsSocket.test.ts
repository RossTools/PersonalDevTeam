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
});
