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
  };
}
