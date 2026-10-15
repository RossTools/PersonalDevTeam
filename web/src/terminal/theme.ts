import type { ITheme } from '@xterm/xterm';

/** Reads a design token from the document (tokens.css is the only place colors live). */
export function cssVar(name: string): string {
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
}

export function terminalTheme(): ITheme {
  return {
    background: cssVar('--pdt-bg'),
    foreground: cssVar('--pdt-fg'),
    cursor: cssVar('--pdt-fg'),
    selectionBackground: cssVar('--pdt-accent-soft'),
    black: cssVar('--pdt-term-black'),
    red: cssVar('--pdt-term-red'),
    green: cssVar('--pdt-term-green'),
    yellow: cssVar('--pdt-term-yellow'),
    blue: cssVar('--pdt-term-blue'),
    magenta: cssVar('--pdt-term-magenta'),
    cyan: cssVar('--pdt-term-cyan'),
    white: cssVar('--pdt-term-white'),
    brightBlack: cssVar('--pdt-term-bright-black'),
    brightRed: cssVar('--pdt-term-bright-red'),
    brightGreen: cssVar('--pdt-term-bright-green'),
    brightYellow: cssVar('--pdt-term-bright-yellow'),
    brightBlue: cssVar('--pdt-term-bright-blue'),
    brightMagenta: cssVar('--pdt-term-bright-magenta'),
    brightCyan: cssVar('--pdt-term-bright-cyan'),
    brightWhite: cssVar('--pdt-term-bright-white'),
  };
}
