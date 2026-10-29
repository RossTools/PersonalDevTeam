import { afterEach, describe, expect, it, vi } from 'vitest';
import { onThemeChange, terminalTheme } from './theme';

const ANSI = ['black', 'red', 'green', 'yellow', 'blue', 'magenta', 'cyan', 'white'] as const;

afterEach(() => {
  document.documentElement.removeAttribute('style');
  document.documentElement.removeAttribute('data-theme');
  vi.unstubAllGlobals();
});

describe('terminalTheme', () => {
  it('maps the base and all 16 ANSI colors to tokens', () => {
    const root = document.documentElement;
    root.style.setProperty('--pdt-bg', '#000001');
    root.style.setProperty('--pdt-fg', '#000002');
    ANSI.forEach((n, i) => {
      root.style.setProperty(`--pdt-term-${n}`, `#0000${i}a`);
      root.style.setProperty(`--pdt-term-bright-${n}`, `#0000${i}b`);
    });
    const t = terminalTheme() as Record<string, string>;
    expect([t.background, t.foreground]).toEqual(['#000001', '#000002']);
    ANSI.forEach((n, i) => {
      expect(t[n]).toBe(`#0000${i}a`);
      expect(t[`bright${n[0].toUpperCase()}${n.slice(1)}`]).toBe(`#0000${i}b`);
    });
  });
});

describe('onThemeChange', () => {
  it('fires on a theme choice or an OS colour scheme change, until unsubscribed', async () => {
    const listeners: (() => void)[] = [];
    vi.stubGlobal('matchMedia', (media: string) => ({
      media,
      matches: false,
      addEventListener: (_: string, cb: () => void) => listeners.push(cb),
      removeEventListener: (_: string, cb: () => void) => listeners.splice(listeners.indexOf(cb), 1),
    }));
    const onChange = vi.fn();
    const stop = onThemeChange(onChange);
    document.documentElement.setAttribute('data-theme', 'dark');
    await vi.waitFor(() => expect(onChange).toHaveBeenCalledTimes(1));
    listeners.forEach((cb) => cb());
    expect(onChange).toHaveBeenCalledTimes(2);
    stop();
    expect(listeners).toEqual([]);
    document.documentElement.removeAttribute('data-theme');
    await Promise.resolve();
    expect(onChange).toHaveBeenCalledTimes(2);
  });
});
