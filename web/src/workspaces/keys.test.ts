import { renderHook } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { actionFor, usePrefixKeys } from './keys';

const press = (key: string, init: KeyboardEventInit = {}) => {
  const e = new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true, ...init });
  document.body.dispatchEvent(e);
  return e;
};

describe('prefix keys', () => {
  it('maps keys to actions', () => {
    expect(actionFor('c')).toEqual({ type: 'newTab' });
    expect(actionFor('3')).toEqual({ type: 'tab', index: 2 });
    expect(actionFor('0')).toBeNull();
    expect(actionFor('w')).toEqual({ type: 'switcher' });
    expect(actionFor('z')).toBeNull();
  });

  it('runs the action for the key after the prefix and hides both from the page', () => {
    const onAction = vi.fn();
    const seen = vi.fn();
    document.body.addEventListener('keydown', seen);
    renderHook(() => usePrefixKeys(onAction));
    expect(press(']', { ctrlKey: true }).defaultPrevented).toBe(true);
    press('Control');
    expect(press('n').defaultPrevented).toBe(true);
    expect(onAction).toHaveBeenCalledWith({ type: 'next' });
    expect(seen).not.toHaveBeenCalledWith(expect.objectContaining({ key: 'n' }));
    // Without the prefix, keys pass through untouched.
    expect(press('n').defaultPrevented).toBe(false);
    expect(onAction).toHaveBeenCalledTimes(1);
    document.body.removeEventListener('keydown', seen);
  });

  it('lets a second prefix through to the terminal, and swallows unbound keys', () => {
    const onAction = vi.fn();
    renderHook(() => usePrefixKeys(onAction));
    press(']', { ctrlKey: true });
    expect(press(']', { ctrlKey: true }).defaultPrevented).toBe(false);
    press(']', { ctrlKey: true });
    expect(press('z').defaultPrevented).toBe(true);
    expect(onAction).not.toHaveBeenCalled();
  });
});
