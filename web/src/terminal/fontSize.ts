// The terminal font size (PDT-115), kept in this browser so a size picked on the phone sticks; every open terminal
// follows a change.
import { useSyncExternalStore } from 'react';
import { clampFontSize, FONT_SIZE } from './keyBar';

const KEY = 'pdt.terminal.fontSize';
const listeners = new Set<() => void>();

export function terminalFontSize(): number {
  try {
    const v = localStorage.getItem(KEY);
    return v === null ? FONT_SIZE.default : clampFontSize(Number(v));
  } catch {
    return FONT_SIZE.default;
  }
}

export function setTerminalFontSize(size: number) {
  const n = clampFontSize(size);
  try {
    if (n === FONT_SIZE.default) localStorage.removeItem(KEY);
    else localStorage.setItem(KEY, String(n));
  } catch {
    // Storage refused (private mode): the size still applies until reload.
  }
  listeners.forEach((l) => l());
}

const subscribe = (l: () => void) => {
  listeners.add(l);
  return () => listeners.delete(l);
};

export function useTerminalFontSize(): number {
  return useSyncExternalStore(subscribe, terminalFontSize);
}

/** The sizes the phone's ⋯ offers. */
export const FONT_SIZES = [10, 11, 12, 13, 14, 15, 16, 18, 20];
