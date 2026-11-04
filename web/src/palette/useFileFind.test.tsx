import { renderHook, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { ReactNode } from 'react';
import { describe, expect, it, vi } from 'vitest';
import { api } from '../api/client';
import { interleave, useFileFind } from './useFileFind';

describe('useFileFind', () => {
  it('takes each checkout\'s best in turn, up to the cap', () => {
    expect(interleave([['a1', 'a2', 'a3'], ['b1'], []], 3)).toEqual(['a1', 'b1', 'a2']);
    expect(interleave([[], []], 5)).toEqual([]);
  });

  it('asks each checkout once the query has 2 characters, and finds nothing while closed', async () => {
    const find = vi.spyOn(api, 'findFiles').mockImplementation(async (projectId) => [`${projectId}.ts`]);
    const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    const wrapper = ({ children }: { children: ReactNode }) => <QueryClientProvider client={qc}>{children}</QueryClientProvider>;
    const checkouts = [{ projectId: 'p1', worktreeId: 'wt1', label: 'PDT-1' }, { projectId: 'p2', worktreeId: null, label: 'Demo' }];
    const { result, rerender } = renderHook(({ q, open }) => useFileFind(checkouts, q, open), { wrapper, initialProps: { q: 'a', open: true } });
    await new Promise((r) => setTimeout(r, 250));
    expect(find).not.toHaveBeenCalled();
    rerender({ q: 'ab', open: true });
    await waitFor(() => expect(result.current.map((f) => f.path)).toEqual(['p1.ts', 'p2.ts']));
    expect(find).toHaveBeenCalledWith('p1', 'wt1', 'ab', expect.anything());
    expect(find).toHaveBeenCalledWith('p2', null, 'ab', expect.anything());
    expect(result.current[0].checkout.label).toBe('PDT-1');
    rerender({ q: 'ab', open: false });
    expect(result.current).toEqual([]);
    find.mockRestore();
  });
});
