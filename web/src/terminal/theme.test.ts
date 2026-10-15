import { afterEach, describe, expect, it } from 'vitest';
import { terminalTheme } from './theme';

const ANSI = ['black', 'red', 'green', 'yellow', 'blue', 'magenta', 'cyan', 'white'] as const;

afterEach(() => document.documentElement.removeAttribute('style'));

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
