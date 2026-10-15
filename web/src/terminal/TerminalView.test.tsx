import { render } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

const h = vi.hoisted(() => ({
  term: null as null | {
    written: unknown[]; dataCb: ((d: string) => void) | null; disposed: boolean; resetCount: number; flush(): void;
  },
  sock: null as null | { handlers: Record<string, (...a: unknown[]) => void>; sent: string[]; disposed: boolean; connected: boolean },
}));

vi.mock('@xterm/xterm', () => ({
  Terminal: class {
    cols = 80;
    rows = 24;
    written: unknown[] = [];
    dataCb: ((d: string) => void) | null = null;
    disposed = false;
    resetCount = 0;
    constructor() { h.term = this; }
    loadAddon() {}
    open() {}
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
vi.mock('./ptySocket', () => ({
  ptyUrl: (id: string) => `ws://test/${id}`,
  PtySocket: class {
    handlers: Record<string, (...a: unknown[]) => void>;
    sent: string[] = [];
    disposed = false;
    connected = false;
    constructor(_url: string, handlers: Record<string, (...a: unknown[]) => void>) {
      this.handlers = handlers;
      h.sock = this;
    }
    connect() { this.connected = true; }
    send(d: string) { this.sent.push(d); }
    resize() {}
    retry() {}
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
});
