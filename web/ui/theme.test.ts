import { afterEach, describe, expect, it, vi } from 'vitest';
import { applyThemePref, readThemePref } from './theme';

afterEach(() => {
  localStorage.clear();
  document.documentElement.removeAttribute('data-theme');
  vi.restoreAllMocks();
});

describe('theme', () => {
  it('applies explicit themes via data-theme and persists', () => {
    applyThemePref('dark');
    expect(document.documentElement.getAttribute('data-theme')).toBe('dark');
    expect(readThemePref()).toBe('dark');
  });

  it('system removes the attribute', () => {
    applyThemePref('light');
    applyThemePref('system');
    expect(document.documentElement.hasAttribute('data-theme')).toBe(false);
    expect(readThemePref()).toBe('system');
  });

  it('ignores corrupted stored values', () => {
    localStorage.setItem('pdt.theme', 'purple');
    expect(readThemePref()).toBe('system');
  });

  it('survives storage that throws', () => {
    vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => { throw new Error('blocked'); });
    vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => { throw new Error('blocked'); });
    expect(readThemePref()).toBe('system');
    expect(() => applyThemePref('dark')).not.toThrow();
    expect(document.documentElement.getAttribute('data-theme')).toBe('dark');
  });
});
