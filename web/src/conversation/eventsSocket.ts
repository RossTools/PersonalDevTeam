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
}

export interface EventsSocketOptions {
  baseDelayMs: number;
  maxDelayMs: number;
  /** A connection must stay open this long before the backoff starts over. */
  stableMs: number;
}

const DEFAULTS: EventsSocketOptions = { baseDelayMs: 500, maxDelayMs: 5000, stableMs: 5000 };

/** `ws(s)://<host>/api/sessions/<id>/events?after=<seq>`. */
export function eventsUrl(sessionId: string, after: number, loc: Pick<Location, 'protocol' | 'host'> = window.location): string {
  const scheme = loc.protocol === 'https:' ? 'wss' : 'ws';
  return `${scheme}://${loc.host}/api/sessions/${encodeURIComponent(sessionId)}/events?after=${after}`;
}

/**
 * A conversation's event stream: reconnects with backoff, forever, each time asking only for what came after the
 * highest seq the owner has (`after()`), until disposed.
 */
export class EventsSocket {
  private sock: EventsSocketLike | null = null;
  private attempt = 0;
  private timer: ReturnType<typeof setTimeout> | null = null;
  private disposed = false;
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
    s.onopen = () => {
      if (this.sock !== s) return;
      openedAt = Date.now();
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
      this.h.onMessage(msg);
    };
    s.onclose = () => {
      if (this.sock !== s) return;
      this.sock = null;
      if (this.disposed) return;
      this.h.onStatus?.('reconnecting');
      if (openedAt !== null && Date.now() - openedAt >= this.opts.stableMs) this.attempt = 0;
      const delay = Math.min(this.opts.maxDelayMs, this.opts.baseDelayMs * 2 ** this.attempt);
      this.attempt++;
      this.timer = setTimeout(() => this.connect(), delay);
    };
    this.sock = s;
  }

  dispose(): void {
    this.disposed = true;
    if (this.timer) clearTimeout(this.timer);
    const s = this.sock;
    this.sock = null;
    s?.close();
  }
}
