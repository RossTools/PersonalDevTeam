import { useEffect, useRef, useState } from 'react';
import { Terminal } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import { ClipboardAddon, type ClipboardSelectionType, type IClipboardProvider } from '@xterm/addon-clipboard';
import { WebLinksAddon } from '@xterm/addon-web-links';
import '@xterm/xterm/css/xterm.css';
import type { SessionStatus } from '../api/gen/SessionStatus';
import { PtySocket, ptyUrl, type PtyStatus } from './ptySocket';
import { cssVar, terminalTheme } from './theme';
import { NEWLINE, isShiftEnter, promptSequence, shouldOpenLink } from './input';
import { Button } from '@ui';

/** The least time between two automatic reattach attempts of one terminal. */
const RETRY_EVERY_MS = 5000;

const isTouch = () => typeof matchMedia !== 'undefined' && matchMedia('(pointer: coarse)').matches;

const openLink = (e: MouseEvent, uri: string) => {
  if (shouldOpenLink(e, uri, isTouch())) window.open(uri, '_blank', 'noopener,noreferrer');
};

/**
 * Programs copy by sending OSC 52 (Claude Code does on every copy, alongside its own pbcopy on the host); this puts
 * the text on the viewer's clipboard. Reading is refused: a program in the terminal must not see the clipboard.
 */
const clipboard: IClipboardProvider = {
  readText: () => '',
  writeText: async (_sel: ClipboardSelectionType, text: string) => {
    try {
      await navigator.clipboard.writeText(text);
    } catch {
      // No permission (e.g. Safari without a recent click, or a non-https origin); nothing to fall back to.
    }
  },
};

/**
 * `sessionStatus` is the daemon's view of the session (from the polled list). When it turns `running` while this
 * view is detached from a finished process, the view reattaches by itself (e.g. after Restart or an auto-resume).
 */
export function TerminalView({ sessionId, url, sessionStatus, polledAt, composing = false, onComposeClose }: {
  sessionId: string;
  /** WebSocket URL; defaults to the session's PTY URL. */
  url?: string;
  sessionStatus?: SessionStatus;
  /** Changes with every poll of the session list (e.g. the query's `dataUpdatedAt`), so the status is re-checked. */
  polledAt?: number;
  /** Shows a multi-line text box whose text is sent to the program as one prompt. */
  composing?: boolean;
  onComposeClose?: () => void;
}) {
  const host = useRef<HTMLDivElement>(null);
  const sockRef = useRef<PtySocket | null>(null);
  const termRef = useRef<Terminal | null>(null);
  const [status, setStatus] = useState<PtyStatus>('connecting');

  useEffect(() => {
    const el = host.current!;
    const term = new Terminal({
      fontFamily: cssVar('--pdt-font-mono') || 'monospace',
      fontSize: 13,
      cursorBlink: true,
      scrollback: 5000,
      theme: terminalTheme(),
      // Option+drag selects even while the program captures the mouse (Claude Code does).
      macOptionClickForcesSelection: true,
      // OSC 8 hyperlinks (Claude Code sends them for URLs because pdtd sets FORCE_HYPERLINK), wrapped lines included.
      linkHandler: { activate: openLink, allowNonHttpProtocols: false },
    });
    termRef.current = term;
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.loadAddon(new WebLinksAddon(openLink));
    term.loadAddon(new ClipboardAddon(undefined, clipboard));
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
    term.attachCustomKeyEventHandler((e) => {
      if (!isShiftEnter(e)) return true;
      // Handle keydown only; keypress and keyup of the same key are swallowed so xterm doesn't also send `\r`.
      if (e.type === 'keydown') {
        e.preventDefault();
        if (!replaying()) sock.send(NEWLINE);
      }
      return false;
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
      termRef.current = null;
    };
  }, [sessionId, url]);

  // Send a raw byte sequence to the PTY (from the on-screen key bar), keeping the terminal focused so the phone's
  // soft keyboard stays up.
  const sendKey = (seq: string) => {
    sockRef.current?.send(seq);
    termRef.current?.focus();
  };

  const sendPrompt = (text: string) => {
    const term = termRef.current;
    if (!term) return;
    sockRef.current?.send(promptSequence(text, term.modes.bracketedPasteMode));
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
        <div ref={host} data-testid="terminal" className="absolute inset-0 p-1" />
        {status === 'reconnecting' && <p className="absolute right-2 top-1 text-xs text-waiting">Reconnecting…</p>}
        {status === 'ended' && <p className="absolute right-2 top-1 text-xs text-muted">Session ended.</p>}
      </div>
      {composing && (
        <Composer
          onSend={sendPrompt}
          onClose={() => {
            onComposeClose?.();
            termRef.current?.focus();
          }}
        />
      )}
      <KeyBar onKey={sendKey} />
    </div>
  );
}

/** A multi-line prompt box: Enter adds a line, Cmd/Ctrl+Enter (or Send) sends, Escape closes. */
function Composer({ onSend, onClose }: { onSend: (text: string) => void; onClose: () => void }) {
  const [text, setText] = useState('');
  const send = () => {
    if (!text.trim()) return;
    onSend(text);
    setText('');
  };
  return (
    <div className="flex shrink-0 items-end gap-2 border-t border-border bg-surface p-2">
      <textarea
        aria-label="Prompt"
        autoFocus
        rows={3}
        value={text}
        placeholder="Write a prompt. Enter adds a line; ⌘/Ctrl+Enter sends."
        onChange={(e) => setText(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
            e.preventDefault();
            send();
          } else if (e.key === 'Escape') {
            e.preventDefault();
            onClose();
          }
        }}
        className="min-h-16 flex-1 resize-y rounded-pdt border border-border bg-bg px-2 py-1.5 font-mono text-sm text-fg outline-none focus-visible:ring-2 focus-visible:ring-ring"
      />
      <div className="flex flex-col gap-1">
        <Button variant="primary" disabled={!text.trim()} onClick={send}>Send</Button>
        <Button variant="ghost" onClick={onClose}>Close</Button>
      </div>
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
