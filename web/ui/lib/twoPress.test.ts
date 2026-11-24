import { act, renderHook } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { twoPressTiming, useTwoPress } from './twoPress';

describe('useTwoPress', () => {
  afterEach(() => {
    twoPressTiming.confirmAfterMs = 0;
  });

  it('arms on the first press and acts on a second, not inside a double-click', () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'Date'] });
    twoPressTiming.confirmAfterMs = 300;
    const act1 = vi.fn();
    const { result } = renderHook(() => useTwoPress());
    act(() => result.current.press('close', act1));
    expect(result.current.armed).toBe('close');
    act(() => result.current.press('close', act1));
    expect(act1).not.toHaveBeenCalled();
    act(() => vi.advanceTimersByTime(300));
    act(() => result.current.press('close', act1));
    expect(act1).toHaveBeenCalledOnce();
    expect(result.current.armed).toBeNull();
    expect(result.current.lapsed).toBe(false);
  });

  it('arming another action disarms the first', () => {
    const run = vi.fn();
    const { result } = renderHook(() => useTwoPress());
    act(() => result.current.press('a', run));
    act(() => result.current.press('b', run));
    expect(result.current.armed).toBe('b');
    expect(run).not.toHaveBeenCalled();
  });

  it('disarms after a few seconds and says it lapsed for a few more', () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'Date'] });
    const { result } = renderHook(() => useTwoPress());
    act(() => result.current.press('a', () => {}));
    act(() => vi.advanceTimersByTime(twoPressTiming.disarmMs));
    expect(result.current.armed).toBeNull();
    expect(result.current.lapsed).toBe(true);
    act(() => vi.advanceTimersByTime(twoPressTiming.disarmMs));
    expect(result.current.lapsed).toBe(false);
  });
});
