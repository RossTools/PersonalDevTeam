import type { ConversationServerMessage } from '../api/gen/ConversationServerMessage';

/** The subset of WebSocket that EventsSocket uses (lets tests supply a fake). */
export interface EventsSocketLike {
  readyState: number;
  onopen: ((ev: Event) => void) | null;
  onmessage: ((ev: MessageEvent) => void) | null;
  onclose: ((ev: CloseEvent) => void) | null;
  close(): void;
}

export type EventsStatus = 'connecting' | 'open' | 'reconnecting';

export interface EventsHandlers {
  /** Each connection opening, before the server replays what came after `after`. */
  onOpen(): void;
  onMessage(msg: ConversationServerMessage): void;
  onStatus?(status: EventsStatus): void;
  /** A connection has not finished replaying (`ready`) `readyMs` after it was made: it may still. */
  onSlow?(): void;
}

export interface EventsSocketOptions {
  baseDelayMs: number;
  maxDelayMs: number;
  /** A connection must stay open this long before the backoff starts over. */
  stableMs: number;
  /** How long a connection may take to replay before the owner hears it is slow (`onSlow`). */
  readyMs: number;
  /** How long a connection may go without a frame before `ready` (opening or replaying) before it is dropped. */
  stallMs: number;
}

const DEFAULTS: EventsSocketOptions = { baseDelayMs: 500, maxDelayMs: 5000, stableMs: 5000, readyMs: 10_000, stallMs: 8000 };

/** `ws(s)://<host>/api/sessions/<id>/events?after=<seq>`. */
export function eventsUrl(sessionId: string, after: number, loc: Pick<Location, 'protocol' | 'host'> = window.location): string {
  const scheme = loc.protocol === 'https:' ? 'wss' : 'ws';
  return `${scheme}://${loc.host}/api/sessions/${encodeURIComponent(sessionId)}/events?after=${after}`;
}

/**
 * A conversation's event stream: reconnects with backoff, forever, each time asking only for what came after the
 * highest seq the owner has (`after()`), until disposed. A connection that stops sending before `ready` (a stalled
 * link, a socket that never opens or closes) is dropped and made again, so a replay never waits forever (PDT-630).
 */
export class EventsSocket {
  private sock: EventsSocketLike | null = null;
  private attempt = 0;
  private timer: ReturnType<typeof setTimeout> | null = null;
  private disposed = false;
  /** Until the current connection's `ready`: the time limits on it. */
  private slowTimer: ReturnType<typeof setTimeout> | null = null;
  private stallTimer: ReturnType<typeof setTimeout> | null = null;
  private readonly opts: EventsSocketOptions;

  constructor(
    private readonly url: (after: number) => string,
    private readonly after: () => number,
    private readonly h: EventsHandlers,
    private readonly factory: (url: string) => EventsSocketLike = (u) => new WebSocket(u),
    opts: Partial<EventsSocketOptions> = {},
  ) {
    this.opts = { ...DEFAULTS, ...opts };
  }

  connect(): void {
    if (this.disposed) return;
    if (this.attempt === 0 && !this.sock) this.h.onStatus?.('connecting');
    const s = this.factory(this.url(this.after()));
    let openedAt: number | null = null;
    let ready = false;
    const stalled = () => {
      if (this.stallTimer) clearTimeout(this.stallTimer);
      this.stallTimer = setTimeout(() => this.drop(s, openedAt), this.opts.stallMs);
    };
    this.clearWatch();
    this.slowTimer = setTimeout(() => {
      this.slowTimer = null;
      if (this.sock === s) this.h.onSlow?.();
    }, this.opts.readyMs);
    stalled();
    s.onopen = () => {
      if (this.sock !== s) return;
      openedAt = Date.now();
      stalled();
      this.h.onOpen();
      this.h.onStatus?.('open');
    };
    s.onmessage = (ev) => {
      if (this.sock !== s || typeof ev.data !== 'string') return;
      let msg: ConversationServerMessage;
      try {
        msg = JSON.parse(ev.data) as ConversationServerMessage;
      } catch {
        return;
      }
      if (!ready) {
        if (msg.type === 'ready') {
          ready = true;
          this.clearWatch();
        } else {
          stalled();
        }
      }
      this.h.onMessage(msg);
    };
    s.onclose = () => this.drop(s, openedAt);
    this.sock = s;
  }

  /** Drops the connection now and makes a new one at once (the user's Retry). */
  retry(): void {
    const s = this.sock;
    if (this.disposed) return;
    this.attempt = 0;
    if (s) {
      this.drop(s, null, 0);
    } else {
      if (this.timer) clearTimeout(this.timer);
      this.connect();
    }
  }

  /** Connection `s` is over (closed, or stalled): the next one is made after the backoff, or `delay`. */
  private drop(s: EventsSocketLike, openedAt: number | null, delay?: number): void {
    if (this.sock !== s) return;
    this.sock = null;
    this.clearWatch();
    // A stalled socket may take as long to close as it took to stall: nothing it sends counts any more.
    s.onopen = s.onmessage = s.onclose = null;
    s.close();
    if (this.disposed) return;
    this.h.onStatus?.('reconnecting');
    if (openedAt !== null && Date.now() - openedAt >= this.opts.stableMs) this.attempt = 0;
    const wait = delay ?? Math.min(this.opts.maxDelayMs, this.opts.baseDelayMs * 2 ** this.attempt);
    this.attempt++;
    if (this.timer) clearTimeout(this.timer);
    this.timer = setTimeout(() => this.connect(), wait);
  }

  private clearWatch(): void {
    if (this.slowTimer) clearTimeout(this.slowTimer);
    if (this.stallTimer) clearTimeout(this.stallTimer);
    this.slowTimer = this.stallTimer = null;
  }

  dispose(): void {
    this.disposed = true;
    this.clearWatch();
    if (this.timer) clearTimeout(this.timer);
    const s = this.sock;
    this.sock = null;
    s?.close();
  }
}
