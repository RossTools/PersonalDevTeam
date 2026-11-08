// The notification stream and the Updates poll: slow while the stream is connected, live while it isn't.
import { act, renderHook } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { useMemo, type ReactNode } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { POLL, streamPoll } from '../common/poll';
import { toast } from '@ui/Toaster';
import { notification } from './fixtures';
import { useNotificationStream } from './useNotificationStream';

class FakeEventSource {
  static last: FakeEventSource | null = null;
  static readonly OPEN = 1;
  readyState = 0;
  onopen: (() => void) | null = null;
  onerror: (() => void) | null = null;
  closed = false;
  constructor() { FakeEventSource.last = this; }
  addEventListener(_type: string, _l: (e: MessageEvent<string>) => void) {}
  close() { this.closed = true; }
}

/** Delivers rows to the listeners the stream adds. */
class ListeningEventSource extends FakeEventSource {
  private listeners = new Map<string, (e: MessageEvent<string>) => void>();
  override addEventListener(type: string, l: (e: MessageEvent<string>) => void) { this.listeners.set(type, l); }
  emit(type: string, data: unknown) { this.listeners.get(type)?.(new MessageEvent(type, { data: JSON.stringify(data) })); }
}

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

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

  it('closes a toast when the item it announces comes on screen', () => {
    const sources: ListeningEventSource[] = [];
    vi.stubGlobal('EventSource', class extends ListeningEventSource { constructor() { super(); sources.push(this); } });
    const shown = vi.spyOn(toast, 'success').mockReturnValue('toast-1');
    const dismissed = vi.spyOn(toast, 'dismiss').mockImplementation(() => 'toast-1');
    const qc = new QueryClient();
    const wrapper = ({ children }: { children: ReactNode }) => <QueryClientProvider client={qc}>{children}</QueryClientProvider>;
    const { rerender } = renderHook(({ inbox }: { inbox: string | null }) =>
      useNotificationStream(() => {}, useMemo(() => ({ session: null, inbox }), [inbox])),
    { wrapper, initialProps: { inbox: null as string | null } });
    const row = notification('n1', { kind: 'inbox.approval', show: 'toast', inboxItemId: 'item-1', target: { kind: 'inbox', id: 'item-1' } });
    act(() => sources[0].emit('created', row));
    expect(shown).toHaveBeenCalledOnce();

    rerender({ inbox: 'other' });
    expect(dismissed).not.toHaveBeenCalled();
    rerender({ inbox: 'item-1' });
    expect(dismissed).toHaveBeenCalledWith('toast-1');
  });
});

