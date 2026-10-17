import { useEffect, useRef, useState } from 'react';
import { Terminal } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import '@xterm/xterm/css/xterm.css';
import type { SessionStatus } from '../api/gen/SessionStatus';
import { PtySocket, ptyUrl, type PtyStatus } from './ptySocket';
import { cssVar, terminalTheme } from './theme';

/** The least time between two automatic reattach attempts of one terminal. */
const RETRY_EVERY_MS = 5000;

/**
 * `sessionStatus` is the daemon's view of the session (from the polled list). When it turns `running` while this
 * view is detached from a finished process, the view reattaches by itself (e.g. after Restart or an auto-resume).
 */
export function TerminalView({ sessionId, url, sessionStatus, polledAt }: {
  sessionId: string;
  /** WebSocket URL; defaults to the session's PTY URL. */
  url?: string;
  sessionStatus?: SessionStatus;
  /** Changes with every poll of the session list (e.g. the query's `dataUpdatedAt`), so the status is re-checked. */
  polledAt?: number;
}) {
  const host = useRef<HTMLDivElement>(null);
  const sockRef = useRef<PtySocket | null>(null);
  const termRef = useRef<Terminal | null>(null);
  const [status, setStatus] = useState<PtyStatus>('connecting');

  useEffect(() => {
    const el = host.current!;
    const fontFamily = cssVar('--pdt-font-mono') || 'monospace';
    const fontSize = 13;
    const term = new Terminal({
      fontFamily,
      fontSize,
      cursorBlink: true,
      scrollback: 5000,
      theme: terminalTheme(),
    });
    termRef.current = term;
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.open(el);
    // The scrollback replayed on each connect contains terminal queries (cursor position, device attributes,
    // colours) that xterm answers through onData. Those answers were already given once, so they must not reach
    // the PTY again: input is held back until the server says the replay is over and xterm has parsed all of it.
    let replay = { ready: true, pending: 0 };
    const replaying = () => !replay.ready || replay.pending > 0;
    const sock = new PtySocket(url ?? ptyUrl(sessionId), {
      onData: (bytes, isReplay) => {
        if (!isReplay) {
          term.write(bytes);
          return;
        }
        const r = replay;
        r.pending++;
        term.write(bytes, () => r.pending--);
      },
      onReset: () => {
        term.reset();
        replay = { ready: false, pending: 0 };
      },
      onReady: () => {
        replay.ready = true;
      },
      onExit: (code) => term.write(`\r\n\x1b[2m[process exited with code ${code}]\x1b[0m\r\n`),
      onStatus: setStatus,
    });
    sockRef.current = sock;
    // Refits at most once per frame, and tells the PTY only when the grid really changed: a resize the PTY sees
    // makes the program redraw, and a redraw must never feed back into another layout change.
    let frame = 0;
    let sent = '';
    const refit = () => {
      try {
        fit.fit();
      } catch {
        // Not laid out yet (hidden panel); the next resize fixes it.
      }
      const size = `${term.cols}x${term.rows}`;
      if (size === sent) return;
      sent = size;
      sock.resize(term.cols, term.rows);
    };
    const scheduleRefit = () => {
      if (frame) return;
      frame = requestAnimationFrame(() => {
        frame = 0;
        refit();
      });
    };
    refit();
    // The web font loads lazily: until it has, xterm measures its cells with a fallback font (a different height),
    // and it measures again only when some later resize forces it, so the grid stops fitting and the layout jumps.
    // Once the font is in, make xterm measure again and refit.
    let disposed = false;
    document.fonts?.load(`${fontSize}px ${fontFamily}`).then(
      () => {
        if (disposed) return;
        term.options.fontFamily = 'monospace';
        term.options.fontFamily = fontFamily;
        scheduleRefit();
      },
      () => {},
    );
    const input = term.onData((d) => {
      if (!replaying()) sock.send(d);
    });
    const observer = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(scheduleRefit);
    observer?.observe(el);
    sock.connect();
    return () => {
      disposed = true;
      observer?.disconnect();
      cancelAnimationFrame(frame);
      input.dispose();
      sock.dispose();
      term.dispose();
      sockRef.current = null;
      termRef.current = null;
    };
  }, [sessionId, url]);

  // Send a raw byte sequence to the PTY (from the on-screen key bar), keeping the terminal focused so the phone's
  // soft keyboard stays up.
  const sendKey = (seq: string) => {
    sockRef.current?.send(seq);
    termRef.current?.focus();
  };

  // Safety net: if the terminal is ended or exited but the latest poll says the session is running, reattach. It is
  // evaluated on every poll (the status may have flipped between two polls and never been seen to change), at most
  // once per RETRY_EVERY_MS so a session that is not really attachable cannot make this loop.
  const statusRef = useRef(status);
  statusRef.current = status;
  const lastRetry = useRef(0);
  useEffect(() => {
    if (sessionStatus !== 'running' || (statusRef.current !== 'ended' && statusRef.current !== 'exited')) return;
    const now = Date.now();
    if (now - lastRetry.current < RETRY_EVERY_MS) return;
    lastRetry.current = now;
    sockRef.current?.retry();
  }, [sessionStatus, polledAt]);

  return (
    <div className="relative flex min-h-0 flex-1 flex-col bg-bg">
      <div className="relative min-h-0 flex-1">
        {/* The padding lives on this wrapper: xterm fits itself to the full height of the element it is opened in,
            padding included, so a padded host makes it stick out and the page scroll (a resize loop on phones). */}
        <div className="absolute inset-0 overflow-hidden p-1">
          <div ref={host} data-testid="terminal" className="h-full w-full" />
        </div>
        {status === 'reconnecting' && <p className="absolute right-2 top-1 text-xs text-waiting">Reconnecting…</p>}
        {status === 'ended' && <p className="absolute right-2 top-1 text-xs text-muted">Session ended.</p>}
      </div>
      <KeyBar onKey={sendKey} />
    </div>
  );
}

/** Keys a phone's soft keyboard can't send, for driving agent prompts and menus. Shown only on touch devices. */
const KEYS: { label: string; aria: string; seq: string }[] = [
  { label: 'Esc', aria: 'Escape', seq: '\x1b' },
  { label: 'Tab', aria: 'Tab', seq: '\t' },
  { label: '^C', aria: 'Control C', seq: '\x03' },
  { label: '↑', aria: 'Up', seq: '\x1b[A' },
  { label: '↓', aria: 'Down', seq: '\x1b[B' },
  { label: '←', aria: 'Left', seq: '\x1b[D' },
  { label: '→', aria: 'Right', seq: '\x1b[C' },
];

function KeyBar({ onKey }: { onKey: (seq: string) => void }) {
  return (
    <div
      role="toolbar"
      aria-label="Terminal keys"
      className="hidden shrink-0 gap-1 overflow-x-auto border-t border-border bg-surface px-1 py-1 pointer-coarse:flex"
    >
      {KEYS.map((k) => (
        <button
          key={k.label}
          type="button"
          aria-label={k.aria}
          // Don't take focus from the terminal, or the soft keyboard would close on each tap.
          onMouseDown={(e) => e.preventDefault()}
          onClick={() => onKey(k.seq)}
          className="min-h-11 min-w-11 shrink-0 rounded-pdt bg-surface-2 px-3 text-sm text-fg active:bg-accent-soft"
        >
          {k.label}
        </button>
      ))}
    </div>
  );
}
