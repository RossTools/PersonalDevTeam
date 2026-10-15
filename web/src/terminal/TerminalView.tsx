import { useEffect, useRef, useState } from 'react';
import { Terminal } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import '@xterm/xterm/css/xterm.css';
import { Button } from '@ui';
import { PtySocket, ptyUrl, type PtyStatus } from './ptySocket';
import { cssVar, terminalTheme } from './theme';

export function TerminalView({ sessionId }: { sessionId: string }) {
  const host = useRef<HTMLDivElement>(null);
  const sockRef = useRef<PtySocket | null>(null);
  const [status, setStatus] = useState<PtyStatus>('connecting');

  useEffect(() => {
    const el = host.current!;
    const term = new Terminal({
      fontFamily: cssVar('--pdt-font-mono') || 'monospace',
      fontSize: 13,
      cursorBlink: true,
      scrollback: 5000,
      theme: terminalTheme(),
    });
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.open(el);
    // The scrollback replayed on each connect contains terminal queries (cursor position, device attributes,
    // colours) that xterm answers through onData. Those answers were already given once, so they must not reach
    // the PTY again: input is held back until the server says the replay is over and xterm has parsed all of it.
    let replay = { ready: true, pending: 0 };
    const replaying = () => !replay.ready || replay.pending > 0;
    const sock = new PtySocket(ptyUrl(sessionId), {
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
    const refit = () => {
      try {
        fit.fit();
      } catch {
        // Not laid out yet (hidden panel); the next resize fixes it.
      }
      sock.resize(term.cols, term.rows);
    };
    refit();
    const input = term.onData((d) => {
      if (!replaying()) sock.send(d);
    });
    const observer = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(refit);
    observer?.observe(el);
    sock.connect();
    return () => {
      observer?.disconnect();
      input.dispose();
      sock.dispose();
      term.dispose();
      sockRef.current = null;
    };
  }, [sessionId]);

  return (
    <div className="relative flex-1 min-h-0 bg-bg">
      <div ref={host} data-testid="terminal" className="absolute inset-0 p-1" />
      {status === 'reconnecting' && <p className="absolute right-2 top-1 text-xs text-waiting">Reconnecting…</p>}
      {status === 'failed' && (
        <div role="alert" className="absolute inset-x-2 top-2 flex items-center gap-2 rounded-pdt border border-border bg-surface p-2 text-xs">
          <span className="flex-1">Could not connect to this terminal.</span>
          <Button onClick={() => sockRef.current?.retry()}>Reconnect</Button>
        </div>
      )}
    </div>
  );
}
