import type { PtyClientMessage } from '../api/gen/PtyClientMessage';
import type { PtyServerMessage } from '../api/gen/PtyServerMessage';

/** The subset of WebSocket that PtySocket uses (lets tests supply a fake). */
export interface SocketLike {
  binaryType: BinaryType;
  readyState: number;
  onopen: ((ev: Event) => void) | null;
  onmessage: ((ev: MessageEvent) => void) | null;
  onclose: ((ev: CloseEvent) => void) | null;
  send(data: string | BufferSource): void;
  close(): void;
}

/** `exited`: the process ended while we were attached. `ended`: the session has no process to attach to. */
export type PtyStatus = 'connecting' | 'open' | 'reconnecting' | 'exited' | 'ended';

export interface PtyHandlers {
  /** `replay` is true for scrollback the server replays on connect (frames before its `ready` message). */
  onData(bytes: Uint8Array, replay: boolean): void;
  /** Called when each connection opens, before the server replays the scrollback. Clear the screen here. */
  onReset(): void;
  /** Called when the server has finished replaying the scrollback; later output is live. */
  onReady?(): void;
  onExit(code: number): void;
  /** The server says the session has no running process; reconnecting stops until `retry()`. */
  onEnded?(): void;
  onStatus?(status: PtyStatus): void;
}

export interface PtySocketOptions {
  baseDelayMs: number;
  maxDelayMs: number;
  /** A connection must stay open this long before the backoff starts over (so a flapping one keeps backing off). */
  stableMs: number;
}

const OPEN = 1;
const DEFAULTS: PtySocketOptions = { baseDelayMs: 500, maxDelayMs: 5000, stableMs: 5000 };

export function ptyUrl(sessionId: string, loc: Pick<Location, 'protocol' | 'host'> = window.location): string {
  const scheme = loc.protocol === 'https:' ? 'wss' : 'ws';
  return `${scheme}://${loc.host}/api/sessions/${encodeURIComponent(sessionId)}/pty`;
}

/**
 * A PTY WebSocket that reconnects with backoff, forever, until the process exits, the server reports the session
 * is not running, or the owner disposes it.
 */
export class PtySocket {
  private sock: SocketLike | null = null;
  private size: { cols: number; rows: number } | null = null;
  private attempt = 0;
  private started = false;
  private timer: ReturnType<typeof setTimeout> | null = null;
  private exited = false;
  private ended = false;
  private disposed = false;
  private readonly encoder = new TextEncoder();
  private readonly opts: PtySocketOptions;

  constructor(
    private readonly url: string,
    private readonly h: PtyHandlers,
    private readonly factory: (url: string) => SocketLike = (u) => new WebSocket(u),
    opts: Partial<PtySocketOptions> = {},
  ) {
    this.opts = { ...DEFAULTS, ...opts };
  }

  connect(): void {
    if (this.disposed || this.exited || this.ended) return;
    this.h.onStatus?.(this.started ? 'reconnecting' : 'connecting');
    this.started = true;
    const s = this.factory(this.url);
    s.binaryType = 'arraybuffer';
    let openedAt: number | null = null;
    let ready = false;
    s.onopen = () => {
      openedAt = Date.now();
      this.h.onReset();
      this.h.onStatus?.('open');
      if (this.size) this.control({ type: 'resize', ...this.size });
    };
    s.onmessage = (ev) => {
      if (typeof ev.data === 'string') {
        const msg = JSON.parse(ev.data) as PtyServerMessage;
        if (msg.type === 'ready') {
          ready = true;
          this.h.onReady?.();
        } else if (msg.type === 'exit') {
          this.exited = true;
          this.h.onStatus?.('exited');
          this.h.onExit(msg.code);
        } else if (msg.type === 'not_running') {
          this.ended = true;
          this.sock = null;
          s.close();
          this.h.onStatus?.('ended');
          this.h.onEnded?.();
        }
        return;
      }
      this.h.onData(new Uint8Array(ev.data as ArrayBuffer), !ready);
    };
    s.onclose = () => {
      // Not the current socket: it was replaced (retry) or ended on purpose.
      if (this.sock !== s) return;
      this.sock = null;
      if (openedAt !== null && Date.now() - openedAt >= this.opts.stableMs) this.attempt = 0;
      this.scheduleReconnect();
    };
    this.sock = s;
  }

  private scheduleReconnect(): void {
    if (this.disposed || this.exited || this.ended) return;
    const delay = Math.min(this.opts.maxDelayMs, this.opts.baseDelayMs * 2 ** this.attempt);
    this.attempt++;
    this.timer = setTimeout(() => this.connect(), delay);
  }

  /** Reconnects now, also after the process exited or the session ended (e.g. it was restarted). */
  retry(): void {
    if (this.disposed) return;
    if (this.timer) clearTimeout(this.timer);
    this.timer = null;
    const old = this.sock;
    this.sock = null;
    old?.close();
    this.exited = false;
    this.ended = false;
    this.attempt = 0;
    this.connect();
  }

  send(text: string): void {
    if (this.sock?.readyState === OPEN) this.sock.send(this.encoder.encode(text));
  }

  resize(cols: number, rows: number): void {
    this.size = { cols, rows };
    this.control({ type: 'resize', cols, rows });
  }

  private control(msg: PtyClientMessage): void {
    if (this.sock?.readyState === OPEN) this.sock.send(JSON.stringify(msg));
  }

  dispose(): void {
    this.disposed = true;
    if (this.timer) clearTimeout(this.timer);
    this.sock?.close();
    this.sock = null;
  }
}
