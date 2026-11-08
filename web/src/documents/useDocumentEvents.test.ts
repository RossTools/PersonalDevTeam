import { renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { fromBase64, useDocumentEvents, VIEW_ID, type DocEvent } from './useDocumentEvents';

/** A stand-in for the browser's EventSource: the test opens it and sends its named events. */
class FakeEventSource {
  static all: FakeEventSource[] = [];
  onopen: (() => void) | null = null;
  closed = false;
  private listeners = new Map<string, ((e: MessageEvent<string>) => void)[]>();
  constructor(readonly url: string) {
    FakeEventSource.all.push(this);
  }
  addEventListener(name: string, fn: (e: MessageEvent<string>) => void) {
    this.listeners.set(name, [...(this.listeners.get(name) ?? []), fn]);
  }
  close() {
    this.closed = true;
  }
  open() {
    this.onopen?.();
  }
  send(name: string, data: string) {
    for (const fn of this.listeners.get(name) ?? []) fn(new MessageEvent(name, { data }));
  }
}
const source = () => FakeEventSource.all[FakeEventSource.all.length - 1];

beforeEach(() => {
  FakeEventSource.all = [];
  vi.stubGlobal('EventSource', FakeEventSource);
});
afterEach(() => vi.unstubAllGlobals());

describe('useDocumentEvents', () => {
  it('listens on the document’s stream as this view and hands on each kind of event', () => {
    const events: DocEvent[] = [];
    renderHook(() => useDocumentEvents('d 1', true, (e) => events.push(e)));
    expect(source().url).toBe(`/api/documents/d%201/events?view=${VIEW_ID}`);
    const sent: DocEvent[] = [
      { type: 'diffs', seq: 4, from: 'v2', data: 'AQI=' },
      { type: 'reload', seq: 5 },
      { type: 'changed', version: 'abc' },
      { type: 'comments' },
      { type: 'charts' },
    ];
    for (const e of sent) source().send(e.type, JSON.stringify(e));
    expect(events).toEqual(sent);
  });

  it('ignores a malformed event and keeps listening', () => {
    const onEvent = vi.fn();
    renderHook(() => useDocumentEvents('d', true, onEvent));
    source().send('reload', '{not json');
    source().send('reload', JSON.stringify({ type: 'reload', seq: 2 }));
    expect(onEvent).toHaveBeenCalledTimes(1);
    expect(onEvent).toHaveBeenCalledWith({ type: 'reload', seq: 2 });
  });

  it('catches up on every reconnect, not on the first open', () => {
    const onOpen = vi.fn();
    renderHook(() => useDocumentEvents('d', true, () => {}, onOpen));
    source().open();
    expect(onOpen).not.toHaveBeenCalled();
    source().open();
    source().open();
    expect(onOpen).toHaveBeenCalledTimes(2);
  });

  it('calls the latest handler without reconnecting', () => {
    const first = vi.fn();
    const second = vi.fn();
    const { rerender } = renderHook(({ fn }) => useDocumentEvents('d', true, fn), { initialProps: { fn: first } });
    rerender({ fn: second });
    expect(FakeEventSource.all).toHaveLength(1);
    source().send('comments', JSON.stringify({ type: 'comments' }));
    expect(first).not.toHaveBeenCalled();
    expect(second).toHaveBeenCalledWith({ type: 'comments' });
  });

  it('connects only while enabled, closes on unmount and reconnects for another document', () => {
    const { rerender, unmount } = renderHook(({ id, on }) => useDocumentEvents(id, on, () => {}), { initialProps: { id: 'a', on: false } });
    expect(FakeEventSource.all).toHaveLength(0);
    rerender({ id: 'a', on: true });
    const a = source();
    rerender({ id: 'b', on: true });
    expect(a.closed).toBe(true);
    const b = source();
    expect(b.url).toContain('/api/documents/b/events');
    unmount();
    expect(b.closed).toBe(true);
  });

  it('does nothing where the browser has no EventSource', () => {
    vi.stubGlobal('EventSource', undefined);
    expect(() => renderHook(() => useDocumentEvents('d', true, () => {}))).not.toThrow();
  });
});

describe('fromBase64', () => {
  it('decodes the stream’s diffs to bytes', () => {
    expect(Array.from(fromBase64('AAH/gA=='))).toEqual([0, 1, 255, 128]);
    expect(fromBase64('')).toHaveLength(0);
  });
});
