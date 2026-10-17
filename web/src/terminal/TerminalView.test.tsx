import { act, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

const h = vi.hoisted(() => ({
  term: null as null | {
    written: unknown[]; dataCb: ((d: string) => void) | null; disposed: boolean; resetCount: number; flush(): void;
    keyHandler: ((e: KeyboardEvent) => boolean) | null; modes: { bracketedPasteMode: boolean };
    selectionCb: (() => void) | null; selection: string;
  },
  clipboard: null as null | { readText(s: string): string; writeText(s: string, t: string): Promise<void> },
  sock: null as null | { handlers: Record<string, (...a: unknown[]) => void>; sent: string[]; disposed: boolean; connected: boolean; retries: number },
}));

vi.mock('@xterm/xterm', () => ({
  Terminal: class {
    cols = 80;
    rows = 24;
    written: unknown[] = [];
    dataCb: ((d: string) => void) | null = null;
    disposed = false;
    resetCount = 0;
    keyHandler: ((e: KeyboardEvent) => boolean) | null = null;
    modes = { bracketedPasteMode: true };
    constructor() { h.term = this; }
    attachCustomKeyEventHandler(fn: (e: KeyboardEvent) => boolean) { this.keyHandler = fn; }
    selectionCb: (() => void) | null = null;
    selection = '';
    onSelectionChange(cb: () => void) { this.selectionCb = cb; return { dispose() {} }; }
    hasSelection() { return this.selection !== ''; }
    getSelection() { return this.selection; }
    loadAddon() {}
    open() {}
    focus() {}
    callbacks: (() => void)[] = [];
    write(d: unknown, cb?: () => void) { this.written.push(d); if (cb) this.callbacks.push(cb); }
    /** Simulates xterm finishing parsing everything written so far. */
    flush() { this.callbacks.splice(0).forEach((cb) => cb()); }
    reset() { this.resetCount++; }
    onData(cb: (d: string) => void) { this.dataCb = cb; return { dispose() {} }; }
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
    send(d: string) { this.sent.push(d); }
    resize() {}
    retry() { this.retries++; }
    dispose() { this.disposed = true; }
  },
}));

import { TerminalView } from './TerminalView';

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

  it('sends Shift+Enter as a line break instead of Enter', () => {
    render(<TerminalView sessionId="s1" />);
    const ev = (type: string, init: Partial<KeyboardEvent>) =>
      ({ type, key: 'Enter', shiftKey: false, ctrlKey: false, altKey: false, metaKey: false, preventDefault() {}, ...init }) as KeyboardEvent;
    expect(h.term!.keyHandler!(ev('keydown', { shiftKey: true }))).toBe(false);
    expect(h.term!.keyHandler!(ev('keypress', { shiftKey: true }))).toBe(false);
    expect(h.term!.keyHandler!(ev('keydown', {}))).toBe(true);
    expect(h.sock!.sent).toEqual(['\x1b\r']);
  });

  it('sends a composed prompt as one paste followed by Enter', () => {
    const onClose = vi.fn();
    render(<TerminalView sessionId="s1" composing onComposeClose={onClose} />);
    const box = screen.getByRole('textbox', { name: 'Prompt' });
    expect(screen.getByRole('button', { name: 'Send' })).toBeDisabled();
    fireEvent.change(box, { target: { value: 'fix it\nthen test' } });
    fireEvent.keyDown(box, { key: 'Enter', metaKey: true });
    expect(h.sock!.sent).toEqual(['\x1b[200~fix it\rthen test\x1b[201~\r']);
    expect(box).toHaveValue('');
    fireEvent.keyDown(box, { key: 'Escape' });
    expect(onClose).toHaveBeenCalled();
  });

  it('copies OSC 52 text to the clipboard but never reveals the clipboard', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true });
    render(<TerminalView sessionId="s1" />);
    await h.clipboard!.writeText('c', 'copied');
    expect(writeText).toHaveBeenCalledWith('copied');
    expect(h.clipboard!.readText('c')).toBe('');
  });

  it('copies a mouse selection to the clipboard when the mouse is released', () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true });
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
