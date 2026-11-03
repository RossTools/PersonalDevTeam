import { act, renderHook } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { usePageLimit } from './usePageLimit';

describe('usePageLimit', () => {
  it('grows a page at a time and starts over when the key changes', () => {
    const { result, rerender } = renderHook(({ k }) => usePageLimit(10, k), { initialProps: { k: 'a' } });
    expect(result.current.limit).toBe(10);
    expect(result.current.hasMore(9)).toBe(false);
    expect(result.current.hasMore(10)).toBe(true);
    act(() => result.current.more());
    expect(result.current.limit).toBe(20);
    rerender({ k: 'b' });
    expect(result.current.limit).toBe(10);
    act(() => result.current.more());
    act(() => result.current.more());
    expect(result.current.limit).toBe(30);
  });
});
