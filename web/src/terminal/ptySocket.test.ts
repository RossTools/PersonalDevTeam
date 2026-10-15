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
    onData: (b: Uint8Array, replay: boolean) => events.push(`${replay ? 'replay' : 'data'}:${new TextDecoder().decode(b)}`),
    onReset: () => events.push('reset'),
    onReady: () => events.push('ready'),
    onExit: (c: number) => events.push(`exit:${c}`),
    onEnded: () => events.push('ended'),
    onStatus: (s: string) => events.push(`status:${s}`),
  };
  const sock = new PtySocket('ws://x/api/sessions/s/pty', h, () => {
    const s = new FakeSocket();
    sockets.push(s);
    return s;
  }, { baseDelayMs: 100, maxDelayMs: 400, stableMs: 5000 });
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
    sockets[0].text(JSON.stringify({ type: 'ready' }));
    sockets[0].data('hello');
    sock.send('ls\r');
    expect(new TextDecoder().decode(sockets[0].sent[1] as Uint8Array)).toBe('ls\r');
    expect(events).toEqual(['status:connecting', 'reset', 'status:open', 'ready', 'data:hello']);
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
    expect(events.slice(-4)).toEqual(['status:reconnecting', 'reset', 'status:open', 'replay:one-replayed']);
  });

  it('flags frames before ready as replay, per connection', () => {
    const { sock, sockets, events } = setup();
    sock.connect();
    sockets[0].open();
    sockets[0].data('old');
    sockets[0].text(JSON.stringify({ type: 'ready' }));
    sockets[0].data('new');
    sockets[0].drop();
    vi.advanceTimersByTime(100);
    sockets[1].open();
    sockets[1].data('old-again');
    sockets[1].text(JSON.stringify({ type: 'ready' }));
    sockets[1].data('newer');
    expect(events.filter((e) => !e.startsWith('status:'))).toEqual([
      'reset', 'replay:old', 'ready', 'data:new', 'reset', 'replay:old-again', 'ready', 'data:newer',
    ]);
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

  it('never gives up while the server is unreachable, backing off to the cap', () => {
    const { sock, sockets, events } = setup();
    sock.connect();
    const delays: number[] = [];
    for (let i = 0; i < 30; i++) {
      const before = sockets.length;
      sockets[before - 1].drop();
      let waited = 0;
      while (sockets.length === before) {
        vi.advanceTimersByTime(50);
        waited += 50;
      }
      delays.push(waited);
    }
    expect(sockets).toHaveLength(31);
    expect(delays.slice(0, 5)).toEqual([100, 200, 400, 400, 400]);
    expect(Math.max(...delays)).toBe(400);
    expect(events.filter((e) => e === 'status:reconnecting').length).toBe(30);
    expect(events).not.toContain('status:ended');
  });

  it('resets the backoff only after a connection stayed open long enough', () => {
    const { sock, sockets } = setup();
    sock.connect();
    const nextDelay = (): number => {
      const before = sockets.length;
      sockets[before - 1].drop();
      let waited = 0;
      while (sockets.length === before) {
        vi.advanceTimersByTime(10);
        waited += 10;
      }
      return waited;
    };
    expect(nextDelay()).toBe(100);
    expect(nextDelay()).toBe(200);
    // Opens, then is closed by the server right away (e.g. lag-close): the backoff keeps growing.
    sockets.at(-1)!.open();
    expect(nextDelay()).toBe(400);
    sockets.at(-1)!.open();
    vi.advanceTimersByTime(4900);
    expect(nextDelay()).toBe(400);
    // Open for 5 s: the next drop starts over from the base delay.
    sockets.at(-1)!.open();
    vi.advanceTimersByTime(5000);
    expect(nextDelay()).toBe(100);
  });

  it('stops on not_running, reports ended and does not reconnect', () => {
    const { sock, sockets, events } = setup();
    sock.connect();
    sockets[0].open();
    sockets[0].text(JSON.stringify({ type: 'not_running' }));
    expect(sockets[0].closed).toBe(true);
    sockets[0].drop();
    vi.advanceTimersByTime(60_000);
    expect(sockets).toHaveLength(1);
    expect(events.slice(-2)).toEqual(['status:ended', 'ended']);
  });

  it('retry reconnects from ended and from exited, closing any live socket first', () => {
    const { sock, sockets, events } = setup();
    sock.connect();
    sockets[0].open();
    sockets[0].text(JSON.stringify({ type: 'not_running' }));
    sock.retry();
    expect(sockets).toHaveLength(2);
    sockets[1].open();
    sockets[1].text(JSON.stringify({ type: 'ready' }));
    expect(events.at(-1)).toBe('ready');

    sockets[1].text(JSON.stringify({ type: 'exit', code: 0 }));
    sock.retry();
    expect(sockets).toHaveLength(3);
    expect(sockets[1].closed).toBe(true);
    sockets[1].drop(); // the old socket's late close must not start a second connection
    vi.advanceTimersByTime(10_000);
    expect(sockets).toHaveLength(3);

    // A manual retry while a reconnect is pending replaces it.
    sockets[2].drop();
    sock.retry();
    vi.advanceTimersByTime(10_000);
    expect(sockets).toHaveLength(4);
  });

  it('reports reconnecting as soon as the socket closes, not when the retry timer fires', () => {
    const { sock, sockets, events } = setup();
    sock.connect();
    sockets[0].open();
    sockets[0].drop();
    expect(events.at(-1)).toBe('status:reconnecting');
    expect(sockets).toHaveLength(1);
    vi.advanceTimersByTime(100);
    expect(sockets).toHaveLength(2);
    expect(events.filter((e) => e === 'status:reconnecting')).toHaveLength(1);
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
