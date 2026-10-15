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

export type PtyStatus = 'connecting' | 'open' | 'reconnecting' | 'exited' | 'failed';

export interface PtyHandlers {
  /** `replay` is true for scrollback the server replays on connect (frames before its `ready` message). */
  onData(bytes: Uint8Array, replay: boolean): void;
  /** Called when each connection opens, before the server replays the scrollback. Clear the screen here. */
  onReset(): void;
  /** Called when the server has finished replaying the scrollback; later output is live. */
  onReady?(): void;
  onExit(code: number): void;
  onStatus?(status: PtyStatus): void;
}

export interface PtySocketOptions {
  baseDelayMs: number;
  maxDelayMs: number;
  /** Consecutive connections that close without ever opening before giving up (status 'failed'). */
  maxFailures: number;
}

const OPEN = 1;
const DEFAULTS: PtySocketOptions = { baseDelayMs: 500, maxDelayMs: 5000, maxFailures: 8 };

export function ptyUrl(sessionId: string, loc: Pick<Location, 'protocol' | 'host'> = window.location): string {
  const scheme = loc.protocol === 'https:' ? 'wss' : 'ws';
  return `${scheme}://${loc.host}/api/sessions/${encodeURIComponent(sessionId)}/pty`;
}

/** A PTY WebSocket that reconnects with backoff until the process exits or the owner disposes it. */
export class PtySocket {
  private sock: SocketLike | null = null;
  private size: { cols: number; rows: number } | null = null;
  private attempt = 0;
  private failures = 0;
  private timer: ReturnType<typeof setTimeout> | null = null;
  private exited = false;
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
    if (this.disposed || this.exited) return;
    this.h.onStatus?.(this.attempt === 0 ? 'connecting' : 'reconnecting');
    const s = this.factory(this.url);
    s.binaryType = 'arraybuffer';
    let opened = false;
    let ready = false;
    s.onopen = () => {
      opened = true;
      this.attempt = 0;
      this.failures = 0;
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
        }
        return;
      }
      this.h.onData(new Uint8Array(ev.data as ArrayBuffer), !ready);
    };
    s.onclose = () => {
      if (this.sock === s) this.sock = null;
      if (!opened) this.failures++;
      this.scheduleReconnect();
    };
    this.sock = s;
  }

  private scheduleReconnect(): void {
    if (this.disposed || this.exited) return;
    if (this.failures >= this.opts.maxFailures) {
      this.h.onStatus?.('failed');
      return;
    }
    const delay = Math.min(this.opts.maxDelayMs, this.opts.baseDelayMs * 2 ** this.attempt);
    this.attempt++;
    this.timer = setTimeout(() => this.connect(), delay);
  }

  /** Manual reconnect after the socket gave up. */
  retry(): void {
    if (this.timer) clearTimeout(this.timer);
    this.failures = 0;
    this.attempt = 1;
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
