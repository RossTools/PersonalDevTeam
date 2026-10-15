import { useState } from 'react';

export type ThemePref = 'system' | 'light' | 'dark';
const KEY = 'pdt.theme';

export function readThemePref(): ThemePref {
  try {
    const v = localStorage.getItem(KEY);
    if (v === 'light' || v === 'dark') return v;
  } catch {
    // storage unavailable (private window, blocked) — fall back to system
  }
  return 'system';
}

export function applyThemePref(pref: ThemePref): void {
  const root = document.documentElement;
  if (pref === 'system') root.removeAttribute('data-theme');
  else root.setAttribute('data-theme', pref);
  try {
    localStorage.setItem(KEY, pref);
  } catch {
    // non-persistent is acceptable
  }
}

export function useThemePref(): [ThemePref, (p: ThemePref) => void] {
  const [pref, setPref] = useState<ThemePref>(readThemePref);
  return [pref, (p) => { applyThemePref(p); setPref(p); }];
}
