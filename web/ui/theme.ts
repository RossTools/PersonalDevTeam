import { useSyncExternalStore } from 'react';

export type ThemePref = 'system' | 'light' | 'dark';
const KEY = 'pdt.theme';
/** Fired on `window` when the choice changes, so every `useThemePref` (the rail's switch, Settings → Appearance) follows. */
const CHANGED = 'pdt-theme-change';

export function readThemePref(): ThemePref {
  try {
    const v = localStorage.getItem(KEY);
    if (v === 'light' || v === 'dark') return v;
  } catch {
    // storage unavailable (private window, blocked) — fall back to system
  }
  return 'system';
}

/** The choice made in this page, which holds even when storage can't keep it; null until one is made. */
let chosen: ThemePref | null = null;

export function applyThemePref(pref: ThemePref): void {
  const root = document.documentElement;
  if (pref === 'system') root.removeAttribute('data-theme');
  else root.setAttribute('data-theme', pref);
  try {
    localStorage.setItem(KEY, pref);
  } catch {
    // non-persistent is acceptable
  }
  chosen = pref;
  window.dispatchEvent(new Event(CHANGED));
}

function subscribe(onChange: () => void): () => void {
  // Another tab of PDT chose: its choice is the stored one.
  const fromStorage = (e: StorageEvent) => {
    if (e.key !== KEY) return;
    chosen = null;
    onChange();
  };
  window.addEventListener(CHANGED, onChange);
  window.addEventListener('storage', fromStorage);
  return () => {
    window.removeEventListener(CHANGED, onChange);
    window.removeEventListener('storage', fromStorage);
  };
}

const current = (): ThemePref => chosen ?? readThemePref();

/** The theme choice (kept in this browser) and its setter; every caller sees the same one. */
export function useThemePref(): [ThemePref, (p: ThemePref) => void] {
  const pref = useSyncExternalStore(subscribe, current, () => 'system' as ThemePref);
  return [pref, applyThemePref];
}
