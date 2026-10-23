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
  /**
   * Decodes a gzipped scrollback replay. When set, the socket asks the server for one (`?replay=gzip`): over a remote
   * link the replay (up to 512 KB) is most of what opening a terminal costs, and terminal output gzips ~10-30×.
   * `null` asks for the raw replay.
   */
  gunzip: ((bytes: Uint8Array) => Promise<Uint8Array>) | null;
  baseDelayMs: number;
  maxDelayMs: number;
  /** A connection must stay open this long before the backoff starts over (so a flapping one keeps backing off). */
  stableMs: number;
}

const OPEN = 1;
/** gunzip with the browser's DecompressionStream, or null where there is none (then the replay comes raw). */
export const browserGunzip: PtySocketOptions['gunzip'] =
  typeof DecompressionStream === 'undefined'
    ? null
    : async (bytes) => {
        const stream = new Blob([bytes as BlobPart]).stream().pipeThrough(new DecompressionStream('gzip'));
        return new Uint8Array(await new Response(stream).arrayBuffer());
      };

const DEFAULTS: PtySocketOptions = { gunzip: browserGunzip, baseDelayMs: 500, maxDelayMs: 5000, stableMs: 5000 };

/** Adds `replay=gzip` to a PTY WebSocket URL. */
export function withGzipReplay(url: string): string {
  return `${url}${url.includes('?') ? '&' : '?'}replay=gzip`;
}

export function ptyUrl(sessionId: string, loc: Pick<Location, 'protocol' | 'host'> = window.location): string {
  const scheme = loc.protocol === 'https:' ? 'wss' : 'ws';
  return `${scheme}://${loc.host}/api/sessions/${encodeURIComponent(sessionId)}/pty`;
}

export function runPtyUrl(
  runId: string,
  step: 'setup' | 'command',
  loc: Pick<Location, 'protocol' | 'host'> = window.location,
): string {
  const scheme = loc.protocol === 'https:' ? 'wss' : 'ws';
  return `${scheme}://${loc.host}/api/runs/${encodeURIComponent(runId)}/pty/${step}`;
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
    if (!this.started) this.h.onStatus?.('connecting');
    this.started = true;
    const gunzip = this.opts.gunzip;
    const s = this.factory(gunzip ? withGzipReplay(this.url) : this.url);
    s.binaryType = 'arraybuffer';
    let openedAt: number | null = null;
    let ready = false;
    // Messages are handled in arrival order. Decoding the gzipped replay is asynchronous, so while it runs, later
    // messages (`ready`, live output) wait behind it; otherwise everything is handled at once.
    let pending: Promise<void> | null = null;
    const inOrder = (f: () => void | Promise<void>) => {
      const run = () => (this.sock === s ? f() : undefined);
      const next = pending ? pending.then(run) : run();
      if (!next) return;
      const p: Promise<void> = next.catch(() => {}).finally(() => {
        if (pending === p) pending = null;
      });
      pending = p;
    };
    s.onopen = () => {
      openedAt = Date.now();
      this.h.onReset();
      this.h.onStatus?.('open');
      if (this.size) this.control({ type: 'resize', ...this.size });
    };
    s.onmessage = (ev) => inOrder(() => {
      if (typeof ev.data === 'string') {
        const msg = JSON.parse(ev.data) as PtyServerMessage;
        if (msg.type === 'ready') {
          ready = true;
          // A server that held this connection while it waited for the session to start has not seen our size.
          if (this.size) this.control({ type: 'resize', ...this.size });
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
      const bytes = new Uint8Array(ev.data as ArrayBuffer);
      if (ready || !gunzip) {
        this.h.onData(bytes, !ready);
        return;
      }
      return gunzip(bytes).then(
        (raw) => {
          if (this.sock === s) this.h.onData(raw, true);
        },
        () => {
          // A replay that does not decode is dropped; live output still follows.
        },
      );
    });
    s.onclose = () => {
      // Not the current socket: it was replaced (retry) or ended on purpose.
      if (this.sock !== s) return;
      this.sock = null;
      if (this.disposed || this.exited || this.ended) return;
      this.h.onStatus?.('reconnecting');
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
    this.h.onStatus?.('reconnecting');
    this.connect();
  }

  /**
   * Sends input to the PTY. Returns false when it was not sent (not connected: connecting or reconnecting, exited or
   * ended); input is never queued, since keys typed blind could land in whatever the program shows after a reconnect.
   */
  send(text: string): boolean {
    if (this.sock?.readyState !== OPEN) return false;
    this.sock.send(this.encoder.encode(text));
    return true;
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
