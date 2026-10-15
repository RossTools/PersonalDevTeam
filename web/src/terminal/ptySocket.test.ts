import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { PtySocket, ptyUrl, type SocketLike } from './ptySocket';

class FakeSocket implements SocketLike {
  binaryType: BinaryType = 'blob';
  readyState = 0;
  sent: unknown[] = [];
  closed = false;
  onopen: ((ev: Event) => void) | null = null;
  onmessage: ((ev: MessageEvent) => void) | null = null;
  onclose: ((ev: CloseEvent) => void) | null = null;
  send(data: string | ArrayBufferLike | ArrayBufferView) { this.sent.push(data); }
  close() { this.closed = true; }
  open() { this.readyState = 1; this.onopen?.(new Event('open')); }
  data(bytes: string) { this.onmessage?.({ data: new TextEncoder().encode(bytes).buffer } as MessageEvent); }
  text(s: string) { this.onmessage?.({ data: s } as MessageEvent); }
  drop() { this.readyState = 3; this.onclose?.({ code: 1006 } as CloseEvent); }
}

function setup() {
  const sockets: FakeSocket[] = [];
  const events: string[] = [];
  const h = {
    onData: (b: Uint8Array) => events.push(`data:${new TextDecoder().decode(b)}`),
    onReset: () => events.push('reset'),
    onExit: (c: number) => events.push(`exit:${c}`),
    onStatus: (s: string) => events.push(`status:${s}`),
  };
  const sock = new PtySocket('ws://x/api/sessions/s/pty', h, () => {
    const s = new FakeSocket();
    sockets.push(s);
    return s;
  }, { baseDelayMs: 100, maxDelayMs: 400, maxFailures: 3 });
  return { sock, sockets, events };
}

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

describe('ptyUrl', () => {
  it('uses ws or wss to match the page', () => {
    expect(ptyUrl('a b', { protocol: 'http:', host: '127.0.0.1:7420' })).toBe('ws://127.0.0.1:7420/api/sessions/a%20b/pty');
    expect(ptyUrl('s', { protocol: 'https:', host: 'box.ts.net' })).toBe('wss://box.ts.net/api/sessions/s/pty');
  });
});

describe('PtySocket', () => {
  it('streams data, sends input as bytes and resize as JSON', () => {
    const { sock, sockets, events } = setup();
    sock.resize(100, 30);
    sock.connect();
    sockets[0].open();
    expect(sockets[0].binaryType).toBe('arraybuffer');
    expect(sockets[0].sent).toEqual([JSON.stringify({ type: 'resize', cols: 100, rows: 30 })]);
    sockets[0].data('hello');
    sock.send('ls\r');
    expect(new TextDecoder().decode(sockets[0].sent[1] as Uint8Array)).toBe('ls\r');
    expect(events).toEqual(['status:connecting', 'reset', 'status:open', 'data:hello']);
  });

  it('resets_before_replay_on_reconnect', () => {
    const { sock, sockets, events } = setup();
    sock.connect();
    sockets[0].open();
    sockets[0].data('one');
    sockets[0].drop();
    vi.advanceTimersByTime(100);
    expect(sockets).toHaveLength(2);
    sockets[1].open();
    sockets[1].data('one-replayed');
    expect(events.slice(-4)).toEqual(['status:reconnecting', 'reset', 'status:open', 'data:one-replayed']);
  });

  it('stops after exit and does not reconnect', () => {
    const { sock, sockets, events } = setup();
    sock.connect();
    sockets[0].open();
    sockets[0].text(JSON.stringify({ type: 'exit', code: 2 }));
    sockets[0].drop();
    vi.advanceTimersByTime(10_000);
    expect(sockets).toHaveLength(1);
    expect(events).toContain('exit:2');
    expect(events).toContain('status:exited');
  });

  it('gives up after repeated failures and can retry', () => {
    const { sock, sockets, events } = setup();
    sock.connect();
    for (let i = 0; i < 3; i++) {
      sockets[i].drop();
      vi.advanceTimersByTime(1000);
    }
    expect(events.at(-1)).toBe('status:failed');
    const count = sockets.length;
    sock.retry();
    expect(sockets).toHaveLength(count + 1);
  });

  it('dispose closes and cancels reconnects', () => {
    const { sock, sockets } = setup();
    sock.connect();
    sockets[0].drop();
    sock.dispose();
    vi.advanceTimersByTime(10_000);
    expect(sockets).toHaveLength(1);
  });

  it('drops input while disconnected', () => {
    const { sock, sockets } = setup();
    sock.connect();
    sock.send('x');
    expect(sockets[0].sent).toEqual([]);
  });
});
