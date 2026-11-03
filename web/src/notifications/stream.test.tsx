// The notification stream and the Updates poll: slow while the stream is connected, live while it isn't.
import { act, renderHook } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { ReactNode } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { POLL, streamPoll } from '../common/poll';
import { useNotificationStream } from './useNotificationStream';

class FakeEventSource {
  static last: FakeEventSource | null = null;
  static readonly OPEN = 1;
  readyState = 0;
  onopen: (() => void) | null = null;
  onerror: (() => void) | null = null;
  closed = false;
  constructor() { FakeEventSource.last = this; }
  addEventListener() {}
  close() { this.closed = true; }
}

afterEach(() => vi.unstubAllGlobals());

describe('the notification stream', () => {
  it('slows the Updates poll while connected and refetches them when it connects or drops', () => {
    vi.stubGlobal('EventSource', FakeEventSource);
    const qc = new QueryClient();
    qc.setQueryData(['notifications', 'updates'], {});
    const stale = () => qc.getQueryState(['notifications', 'updates'])?.isInvalidated;
    const wrapper = ({ children }: { children: ReactNode }) => <QueryClientProvider client={qc}>{children}</QueryClientProvider>;
    const { unmount } = renderHook(() => useNotificationStream(() => {}, { session: null, inbox: null }), { wrapper });
    const source = FakeEventSource.last!;
    expect(streamPoll()).toBe(POLL.live);

    act(() => { source.readyState = 1; source.onopen?.(); });
    expect(streamPoll()).toBe(POLL.pushed);
    expect(stale()).toBe(true);

    qc.setQueryData(['notifications', 'updates'], {});
    act(() => { source.readyState = 0; source.onerror?.(); });
    expect(streamPoll()).toBe(POLL.live);
    expect(stale()).toBe(true);

    act(() => { source.readyState = 1; source.onopen?.(); });
    unmount();
    expect(source.closed).toBe(true);
    expect(streamPoll()).toBe(POLL.live);
  });
});
