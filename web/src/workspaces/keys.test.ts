import { renderHook } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { actionFor, KEY_HELP, usePrefixKeys } from './keys';

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

  it('opens the command palette with k, and lists it in the help', () => {
    expect(actionFor('k')).toEqual({ type: 'palette' });
    expect(KEY_HELP.find(([k]) => k === 'k')?.[1]).toMatch(/Command palette/);
  });

  it('opens the backlog with l, and lists it in the help', () => {
    expect(actionFor('l')).toEqual({ type: 'backlog' });
    expect(KEY_HELP.find(([k]) => k === 'l')?.[1]).toMatch(/Tickets/);
  });

  it('opens the board with b, and lists it in the help', () => {
    expect(actionFor('b')).toEqual({ type: 'board' });
    expect(KEY_HELP.find(([k]) => k === 'b')?.[1]).toMatch(/board/);
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
