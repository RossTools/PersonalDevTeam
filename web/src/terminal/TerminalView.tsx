import { useEffect, useImperativeHandle, useRef, useState, type Ref } from 'react';
import { Terminal } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import { ClipboardAddon, type ClipboardSelectionType, type IClipboardProvider } from '@xterm/addon-clipboard';
import { WebLinksAddon } from '@xterm/addon-web-links';
import '@xterm/xterm/css/xterm.css';
import type { SessionStatus } from '../api/gen/SessionStatus';
import { PtySocket, ptyUrl, type PtyStatus } from './ptySocket';
import { cssVar, onThemeChange, terminalTheme } from './theme';
import { NEWLINE, isShiftEnter, promptSequence, shouldOpenLink } from './input';
import { linkUnderliner } from './linkSpan';
import { Composer } from '../common/Composer';
import { Button, toast } from '@ui';
import { cn } from '@ui/lib/utils';
import { runPrefixKey } from '../workspaces/keys';
import { applyPending, pressBarKey, SEND_KEYS, tapCtrl, tapPrefix, wordAt, type Pending } from './keyBar';
import { useTerminalFontSize } from './fontSize';

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

/** How long a still finger must stay down to start selecting text, and how far it may wander before that. */
const LONG_PRESS_MS = 450;
const LONG_PRESS_SLOP_PX = 10;

/** What the parent can do to the terminal from its own menu (the phone's session ⋯). */
export interface TerminalHandle {
  /** Pastes the clipboard into the program (as a paste, so bracketed paste applies). Call it inside a click. */
  paste(): void;
}

const clipboard: IClipboardProvider = {
  readText: () => '',
  writeText: (_sel: ClipboardSelectionType, text: string) => copyText(text),
};

/**
 * `sessionStatus` is the daemon's view of the session (from the polled list). When it turns `running` while this
 * view is detached from a finished process, the view reattaches by itself (e.g. after Restart or an auto-resume).
 */
export function TerminalView({ sessionId, url, sessionStatus, polledAt, composing = false, onComposeClose, dimmed = false, readOnly = false, handle }: {
  sessionId: string;
  /** WebSocket URL; defaults to the session's PTY URL. */
  url?: string;
  sessionStatus?: SessionStatus;
  /** Changes with every poll of the session list (e.g. the query's `dataUpdatedAt`), so the status is re-checked. */
  polledAt?: number;
  /** Shows a multi-line text box whose text is sent to the program as one prompt. */
  composing?: boolean;
  onComposeClose?: () => void;
  /** The session has ended: the last screen stays, dimmed, and the parent shows what happened (no "Session ended."). */
  dimmed?: boolean;
  /** Output only (a detached run's log): no typing, no cursor, no key bar. */
  readOnly?: boolean;
  handle?: Ref<TerminalHandle>;
}) {
  const host = useRef<HTMLDivElement>(null);
  const sockRef = useRef<PtySocket | null>(null);
  const termRef = useRef<Terminal | null>(null);
  const [status, setStatus] = useState<PtyStatus>('connecting');
  // Input was typed while the socket was not connected and so never reached the PTY; shown until it reconnects.
  const [dropped, setDropped] = useState(false);
  const fontSize = useTerminalFontSize();
  const fontSizeRef = useRef(fontSize);
  fontSizeRef.current = fontSize;
  const refitRef = useRef<() => void>(() => {});
  // The key bar's armed key (sticky Ctrl, or the ⌃] prefix): a ref for the input handlers, state for the bar.
  const pendingRef = useRef<Pending>(null);
  const [pending, setPendingState] = useState<Pending>(null);
  const setPending = (p: Pending) => {
    pendingRef.current = p;
    setPendingState(p);
  };
  const setPendingRef = useRef(setPending);
  setPendingRef.current = setPending;
  // Whether the terminal has focus (so a phone's keyboard is up), and whether ⌨ put the keyboard away on purpose.
  const [focused, setFocused] = useState(false);
  const keyboardHidden = useRef(false);
  // A touch selection waiting for Copy.
  const [touchSelection, setTouchSelection] = useState(false);

  useEffect(() => {
    const el = host.current!;
    const fontFamily = cssVar('--pdt-font-mono') || 'monospace';
    const fontSize = fontSizeRef.current;
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
      // A negative code is unknown (a detached run's processes outlived the one PDT started).
      onExit: (code) => term.write(`\r\n\x1b[2m[process exited${code >= 0 ? ` with code ${code}` : ''}]\x1b[0m\r\n`),
      onStatus: (s) => {
        setStatus(s);
        if (s === 'open') setDropped(false);
      },
    });
    sockRef.current = sock;
    const sendBytes = (d: string) => {
      if (!sock.send(d)) setDropped(true);
    };
    // Input typed into the terminal (or pasted) goes through the key bar's armed key first.
    const sendInput = (d: string) => {
      const r = applyPending(pendingRef.current, d);
      if (r.pending !== pendingRef.current) setPendingRef.current(r.pending);
      if (r.prefixKey !== undefined) runPrefixKey(r.prefixKey);
      if (r.send !== undefined) sendBytes(r.send);
    };
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
    refitRef.current = scheduleRefit;
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
      if (!replaying()) sendInput(d);
    });
    term.attachCustomKeyEventHandler((e) => {
      if (!isShiftEnter(e)) return true;
      // Handle keydown only; keypress and keyup of the same key are swallowed so xterm doesn't also send `\r`.
      if (e.type === 'keydown') {
        e.preventDefault();
        if (!replaying()) sendBytes(NEWLINE);
      }
      return false;
    });
    const textarea = term.textarea;
    const onFocus = () => {
      keyboardHidden.current = false;
      setFocused(true);
    };
    const onBlur = () => setFocused(false);
    textarea?.addEventListener('focus', onFocus);
    textarea?.addEventListener('blur', onBlur);
    const touch = touchGestures(el, term, () => setTouchSelection(true), () => setTouchSelection(false));
    const observer = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(scheduleRefit);
    observer?.observe(el);
    // xterm paints its own canvas, so a theme change (Light, Dark, or the OS's while on System) needs a new theme.
    const unwatchTheme = onThemeChange(() => {
      term.options.theme = terminalTheme();
    });
    sock.connect();
    return () => {
      disposed = true;
      unwatchTheme();
      observer?.disconnect();
      cancelAnimationFrame(frame);
      document.removeEventListener('mouseup', copySelection);
      textarea?.removeEventListener('focus', onFocus);
      textarea?.removeEventListener('blur', onBlur);
      touch();
      selection.dispose();
      underliner?.dispose();
      input.dispose();
      sock.dispose();
      term.dispose();
      sockRef.current = null;
      termRef.current = null;
      refitRef.current = () => {};
    };
  }, [sessionId, url]);

  useEffect(() => {
    const term = termRef.current;
    if (!term || term.options.fontSize === fontSize) return;
    term.options.fontSize = fontSize;
    refitRef.current();
  }, [fontSize]);

  useEffect(() => {
    const term = termRef.current;
    if (!term) return;
    term.options.disableStdin = readOnly;
    term.options.cursorInactiveStyle = readOnly ? 'none' : 'outline';
    term.options.cursorBlink = !readOnly;
  }, [readOnly]);

  // Keeps the terminal focused after a key bar tap, so the phone's keyboard stays up, unless ⌨ put it away.
  const refocus = () => {
    if (!keyboardHidden.current) termRef.current?.focus();
  };
  const sendBytes = (d: string) => {
    if (!sockRef.current?.send(d)) setDropped(true);
  };
  // Send a key bar key's bytes to the PTY, through the armed key.
  const sendKey = (seq: string) => {
    const r = pressBarKey(pendingRef.current, seq);
    setPending(r.pending);
    if (r.send !== undefined) sendBytes(r.send);
    refocus();
  };
  const onCtrl = () => {
    setPending(tapCtrl(pendingRef.current));
    refocus();
  };
  const onPrefix = () => {
    const r = tapPrefix(pendingRef.current);
    setPending(r.pending);
    if (r.send !== undefined) sendBytes(r.send);
    refocus();
  };
  const toggleKeyboard = () => {
    const term = termRef.current;
    if (!term) return;
    if (focused) {
      keyboardHidden.current = true;
      term.blur();
    } else {
      keyboardHidden.current = false;
      term.focus();
    }
  };

  const copyTouchSelection = () => {
    const term = termRef.current;
    if (term?.hasSelection()) {
      void copyText(term.getSelection()).then(() => toast.success('Copied'));
      term.clearSelection();
    }
    setTouchSelection(false);
  };

  useImperativeHandle(handle, () => ({
    paste: () => {
      if (!navigator.clipboard?.readText) {
        toast.error("This browser can't read the clipboard");
        return;
      }
      navigator.clipboard.readText().then(
        (text) => {
          if (text) termRef.current?.paste(text);
          refocus();
        },
        () => toast.error("Couldn't read the clipboard"),
      );
    },
  }));

  // Throws when the prompt was not sent, so the Composer keeps the text and shows why.
  const sendPrompt = (text: string) => {
    const term = termRef.current;
    if (!term || !sockRef.current?.send(promptSequence(text, term.modes.bracketedPasteMode))) {
      throw new Error('Not connected to the terminal, so the prompt was not sent. Send it again once it reconnects.');
    }
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
        <div className={`absolute inset-0 overflow-hidden p-1${dimmed ? ' opacity-60' : ''}`} data-dimmed={dimmed || undefined}>
          <div ref={host} data-testid="terminal" className="h-full w-full" />
        </div>
        {dropped && (status === 'reconnecting' || status === 'connecting') ? (
          <p role="status" className="absolute right-2 top-1 text-xs text-failed">Not connected: input not sent</p>
        ) : (
          status === 'reconnecting' && <p className="absolute right-2 top-1 text-xs text-waiting">Reconnecting…</p>
        )}
        {status === 'ended' && !dimmed && <p className="absolute right-2 top-1 text-xs text-muted-foreground">Session ended.</p>}
        {touchSelection && (
          <div role="toolbar" aria-label="Selection" className="absolute left-1/2 top-2 flex -translate-x-1/2 gap-1 rounded-pdt border border-border bg-surface p-1 shadow-popover">
            <Button size="sm" variant="primary" onClick={copyTouchSelection}>Copy</Button>
            <Button
              size="sm"
              variant="ghost"
              onClick={() => {
                termRef.current?.clearSelection();
                setTouchSelection(false);
              }}
            >
              Cancel
            </Button>
          </div>
        )}
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
      {!readOnly && (
        <KeyBar
          pending={pending}
          keyboardUp={focused}
          onKey={sendKey}
          onCtrl={onCtrl}
          onPrefix={onPrefix}
          onKeyboard={toggleKeyboard}
        />
      )}
    </div>
  );
}

/**
 * Touch on the terminal: a drag scrolls the scrollback, and a long press selects the word under the finger, then
 * dragging extends the selection; `onSelect` fires when the finger lifts with text selected. Returns the cleanup.
 */
function touchGestures(el: HTMLElement, term: Terminal, onSelect: () => void, onClear: () => void): () => void {
  let start: { x: number; y: number } | null = null;
  let timer = 0;
  let selecting: { col: number; row: number; end: number } | null = null;
  let scrolled = 0;
  let moved = false;

  /** The buffer cell under a point. */
  const cellAt = (x: number, y: number) => {
    const screen = el.querySelector<HTMLElement>('.xterm-screen') ?? el;
    const r = screen.getBoundingClientRect();
    const col = Math.min(term.cols - 1, Math.max(0, Math.floor(((x - r.left) / r.width) * term.cols)));
    const row = Math.min(term.rows - 1, Math.max(0, Math.floor(((y - r.top) / r.height) * term.rows)));
    return { col, row: term.buffer.active.viewportY + row };
  };
  const cellHeight = () => (el.querySelector<HTMLElement>('.xterm-screen') ?? el).getBoundingClientRect().height / term.rows || 16;

  const begin = () => {
    timer = 0;
    if (!start) return;
    const { col, row } = cellAt(start.x, start.y);
    const line = term.buffer.active.getLine(row)?.translateToString(true) ?? '';
    const [a, b] = wordAt(line, col);
    selecting = { col: a, row, end: b };
    term.select(a, row, b - a);
    navigator.vibrate?.(10);
  };
  const extend = (x: number, y: number) => {
    if (!selecting) return;
    const at = cellAt(x, y);
    const anchor = selecting.row * term.cols + selecting.col;
    const here = at.row * term.cols + at.col;
    if (here < anchor) term.select(at.col, at.row, selecting.row * term.cols + selecting.end - here);
    else term.select(selecting.col, selecting.row, Math.max(here + 1, selecting.row * term.cols + selecting.end) - anchor);
  };

  const onStart = (e: TouchEvent) => {
    if (e.touches.length !== 1) return;
    const t = e.touches[0];
    start = { x: t.clientX, y: t.clientY };
    scrolled = 0;
    moved = false;
    selecting = null;
    if (term.hasSelection()) {
      term.clearSelection();
      onClear();
    }
    window.clearTimeout(timer);
    timer = window.setTimeout(begin, LONG_PRESS_MS);
  };
  const onMove = (e: TouchEvent) => {
    if (!start || e.touches.length !== 1) return;
    const t = e.touches[0];
    if (selecting) {
      e.preventDefault();
      e.stopPropagation();
      extend(t.clientX, t.clientY);
      return;
    }
    const dy = t.clientY - start.y;
    if (!moved && Math.hypot(t.clientX - start.x, dy) < LONG_PRESS_SLOP_PX) return;
    moved = true;
    window.clearTimeout(timer);
    timer = 0;
    // Our own scrolling, a line per cell height: a finger moving down shows earlier lines.
    e.preventDefault();
    e.stopPropagation();
    const lines = Math.trunc(dy / cellHeight()) - scrolled;
    if (lines !== 0) {
      term.scrollLines(-lines);
      scrolled += lines;
    }
  };
  const onEnd = (e: TouchEvent) => {
    window.clearTimeout(timer);
    timer = 0;
    if (selecting || moved) {
      // Not a tap: keep xterm from turning it into a click (which would clear the selection or move focus).
      e.preventDefault();
      e.stopPropagation();
    }
    if (selecting && term.hasSelection()) onSelect();
    selecting = null;
    start = null;
  };
  const opts = { capture: true, passive: false } as const;
  el.addEventListener('touchstart', onStart, opts);
  el.addEventListener('touchmove', onMove, opts);
  el.addEventListener('touchend', onEnd, opts);
  el.addEventListener('touchcancel', onEnd, opts);
  return () => {
    window.clearTimeout(timer);
    el.removeEventListener('touchstart', onStart, opts);
    el.removeEventListener('touchmove', onMove, opts);
    el.removeEventListener('touchend', onEnd, opts);
    el.removeEventListener('touchcancel', onEnd, opts);
  };
}

const KEY_CLASS = 'min-h-11 min-w-11 shrink-0 rounded-pdt bg-surface-2 px-3 text-sm text-fg active:bg-accent';

/**
 * Keys a phone's soft keyboard can't send, for driving agent prompts and menus: Esc Tab Ctrl ^C arrows ⌃] ⌨. Shown
 * only on touch devices; scrolls sideways when it doesn't fit, and never takes focus from the terminal.
 */
function KeyBar({ pending, keyboardUp, onKey, onCtrl, onPrefix, onKeyboard }: {
  pending: Pending;
  keyboardUp: boolean;
  onKey: (seq: string) => void;
  onCtrl: () => void;
  onPrefix: () => void;
  onKeyboard: () => void;
}) {
  // Don't take focus from the terminal, or the soft keyboard would close on each tap.
  const keep = (e: { preventDefault(): void }) => e.preventDefault();
  const key = (label: string, aria: string, onClick: () => void, pressed?: boolean) => (
    <button
      key={label}
      type="button"
      aria-label={aria}
      aria-pressed={pressed}
      onMouseDown={keep}
      onClick={onClick}
      className={cn(KEY_CLASS, pressed && 'bg-primary text-primary-foreground active:bg-primary')}
    >
      {label}
    </button>
  );
  const [esc, tab, ctrlC, ...arrows] = SEND_KEYS;
  const send = (k: (typeof SEND_KEYS)[number]) => key(k.label, k.aria, () => onKey(k.seq));
  return (
    <div
      role="toolbar"
      aria-label="Terminal keys"
      className="hidden shrink-0 gap-1 overflow-x-auto border-t border-border bg-surface px-1 py-1 pointer-coarse:flex"
    >
      {send(esc)}
      {send(tab)}
      {key('Ctrl', 'Control', onCtrl, pending === 'ctrl')}
      {send(ctrlC)}
      {arrows.map(send)}
      {key('⌃]', 'PDT prefix (Control ])', onPrefix, pending === 'prefix')}
      {key('⌨', keyboardUp ? 'Hide keyboard' : 'Show keyboard', onKeyboard)}
    </div>
  );
}
