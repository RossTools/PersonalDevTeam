import { renderHook } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { isMac, useMetaK } from './useMetaK';

const press = (init: KeyboardEventInit) => {
  const e = new KeyboardEvent('keydown', { bubbles: true, cancelable: true, ...init });
  document.body.dispatchEvent(e);
  return e;
};

const onPlatform = (platform: string) => vi.spyOn(navigator, 'platform', 'get').mockReturnValue(platform);

afterEach(() => vi.restoreAllMocks());

describe('useMetaK', () => {
  it('opens on ⌘K on macOS and hides the key from the page', () => {
    onPlatform('MacIntel');
    expect(isMac()).toBe(true);
    const onOpen = vi.fn();
    const seen = vi.fn();
    document.body.addEventListener('keydown', seen);
    renderHook(() => useMetaK(onOpen));
    const e = press({ key: 'k', code: 'KeyK', metaKey: true });
    expect(onOpen).toHaveBeenCalledTimes(1);
    expect(e.defaultPrevented).toBe(true);
    expect(seen).not.toHaveBeenCalled();
    document.body.removeEventListener('keydown', seen);
  });

  it('ignores Ctrl+K, plain k and ⌘ with other modifiers on macOS', () => {
    onPlatform('MacIntel');
    const onOpen = vi.fn();
    renderHook(() => useMetaK(onOpen));
    expect(press({ key: 'k', ctrlKey: true }).defaultPrevented).toBe(false);
    expect(press({ key: 'k' }).defaultPrevented).toBe(false);
    expect(press({ key: 'K', metaKey: true, shiftKey: true }).defaultPrevented).toBe(false);
    expect(onOpen).not.toHaveBeenCalled();
  });

  it('does nothing on other platforms, for ⌘K (Windows key) or Ctrl+K', () => {
    onPlatform('Win32');
    expect(isMac()).toBe(false);
    const onOpen = vi.fn();
    renderHook(() => useMetaK(onOpen));
    expect(press({ key: 'k', metaKey: true }).defaultPrevented).toBe(false);
    expect(press({ key: 'k', ctrlKey: true }).defaultPrevented).toBe(false);
    expect(onOpen).not.toHaveBeenCalled();
  });

  it('stops listening on unmount', () => {
    onPlatform('MacIntel');
    const onOpen = vi.fn();
    const { unmount } = renderHook(() => useMetaK(onOpen));
    unmount();
    press({ key: 'k', metaKey: true });
    expect(onOpen).not.toHaveBeenCalled();
  });
});
