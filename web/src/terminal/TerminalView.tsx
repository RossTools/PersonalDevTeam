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
import { linkUnderliner } from './linkSpan';
import { Composer } from '../common/Composer';

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
const copyText = async (text: string) => {
  try {
    await navigator.clipboard.writeText(text);
  } catch {
    // No permission (e.g. Safari without a recent click, or a non-https origin); nothing to fall back to.
  }
};

const clipboard: IClipboardProvider = {
  readText: () => '',
  writeText: (_sel: ClipboardSelectionType, text: string) => copyText(text),
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
    const fontFamily = cssVar('--pdt-font-mono') || 'monospace';
    const fontSize = 13;
    let underliner: ReturnType<typeof linkUnderliner> | null = null;
    const term = new Terminal({
      fontFamily,
      fontSize,
      cursorBlink: true,
      scrollback: 5000,
      theme: terminalTheme(),
      // Option+drag selects even while the program captures the mouse (Claude Code does).
      macOptionClickForcesSelection: true,
      // registerDecoration (the wrapped-link underline) is a proposed API.
      allowProposedApi: true,
      // OSC 8 hyperlinks (Claude Code sends them for URLs because pdtd sets FORCE_HYPERLINK), wrapped lines included.
      // xterm underlines only the hovered row of a wrapped link; the underliner does the other rows.
      linkHandler: {
        activate: openLink,
        hover: (_e, _text, range) => underliner?.show(range.start.x, range.start.y),
        leave: () => underliner?.clear(),
        allowNonHttpProtocols: false,
      },
    });
    termRef.current = term;
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.loadAddon(new WebLinksAddon(openLink));
    term.loadAddon(new ClipboardAddon(undefined, clipboard));
    term.open(el);
    underliner = linkUnderliner(term);
    // Copy on select: when a mouse selection ends, its text goes to the clipboard (inside the click, so browsers
    // allow it). The program never sees this selection, since it doesn't capture the mouse.
    let selectionChanged = false;
    const selection = term.onSelectionChange(() => {
      selectionChanged = true;
    });
    const copySelection = () => {
      if (!selectionChanged) return;
      selectionChanged = false;
      if (term.hasSelection()) void copyText(term.getSelection());
    };
    document.addEventListener('mouseup', copySelection);
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
    term.attachCustomKeyEventHandler((e) => {
      if (!isShiftEnter(e)) return true;
      // Handle keydown only; keypress and keyup of the same key are swallowed so xterm doesn't also send `\r`.
      if (e.type === 'keydown') {
        e.preventDefault();
        if (!replaying()) sock.send(NEWLINE);
      }
      return false;
    });
    const observer = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(scheduleRefit);
    observer?.observe(el);
    sock.connect();
    return () => {
      disposed = true;
      observer?.disconnect();
      cancelAnimationFrame(frame);
      document.removeEventListener('mouseup', copySelection);
      selection.dispose();
      underliner?.dispose();
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
        {/* The padding lives on this wrapper: xterm fits itself to the full height of the element it is opened in,
            padding included, so a padded host makes it stick out and the page scroll (a resize loop on phones). */}
        <div className="absolute inset-0 overflow-hidden p-1">
          <div ref={host} data-testid="terminal" className="h-full w-full" />
        </div>
        {status === 'reconnecting' && <p className="absolute right-2 top-1 text-xs text-waiting">Reconnecting…</p>}
        {status === 'ended' && <p className="absolute right-2 top-1 text-xs text-muted-foreground">Session ended.</p>}
      </div>
      {composing && (
        <Composer
          submitOn="mod-enter"
          mono
          onSend={sendPrompt}
          onCancel={() => {
            onComposeClose?.();
            termRef.current?.focus();
          }}
        />
      )}
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
          className="min-h-11 min-w-11 shrink-0 rounded-pdt bg-surface-2 px-3 text-sm text-fg active:bg-accent"
        >
          {k.label}
        </button>
      ))}
    </div>
  );
}
