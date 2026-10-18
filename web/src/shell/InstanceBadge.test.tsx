import { act, renderHook, screen, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { ReactNode } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { renderWithQuery, stubApi } from '../test-utils';
import { InstanceBadge } from './InstanceBadge';
import { formatTitle, useDevTitle, useInstance } from './useInstance';

const health = (dev: boolean, port: number) => ({
  status: 'ok', name: 'Personal Dev Team', version: '0.1.0-local', profile: dev ? 'dev' : 'stable', port, dev,
});

function wrapper() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return ({ children }: { children: ReactNode }) => <QueryClientProvider client={qc}>{children}</QueryClientProvider>;
}

afterEach(() => vi.unstubAllGlobals());

describe('useInstance', () => {
  it('reads /api/health once', async () => {
    const calls = stubApi({ 'GET /api/health': health(true, 8512) });
    const { result } = renderHook(() => useInstance(), { wrapper: wrapper() });
    await waitFor(() => expect(result.current.data?.port).toBe(8512));
    expect(result.current.data?.dev).toBe(true);
    expect(calls.filter((c) => c.path === '/api/health')).toHaveLength(1);
  });
});

describe('InstanceBadge', () => {
  it('shows DEV and the port on a development instance', async () => {
    stubApi({ 'GET /api/health': health(true, 8512) });
    renderWithQuery(<InstanceBadge />);
    const badge = await screen.findByText('DEV · 8512');
    expect(badge).toHaveAttribute('title', 'Development instance');
  });

  it('renders nothing on the stable instance', async () => {
    const calls = stubApi({ 'GET /api/health': health(false, 7420) });
    renderWithQuery(<InstanceBadge />);
    await waitFor(() => expect(calls).toHaveLength(1));
    await act(async () => {});
    expect(screen.queryByText(/DEV/)).not.toBeInTheDocument();
  });
});

describe('title', () => {
  beforeEach(() => { document.title = 'PDT'; });
  afterEach(() => { document.title = ''; });

  it('combines the attention count and the dev prefix', () => {
    expect(formatTitle('PDT', {})).toBe('PDT');
    expect(formatTitle('PDT', { dev: true })).toBe('[dev] PDT');
    expect(formatTitle('PDT', { attention: 2 })).toBe('(2) PDT');
    expect(formatTitle('PDT', { attention: 2, dev: true })).toBe('(2) [dev] PDT');
  });

  it('prefixes the page title on a development instance and restores it on unmount', async () => {
    stubApi({ 'GET /api/health': health(true, 8512) });
    const { rerender, unmount } = renderHook(({ n }) => useDevTitle(n), { wrapper: wrapper(), initialProps: { n: 0 } });
    await waitFor(() => expect(document.title).toBe('[dev] PDT'));
    rerender({ n: 2 });
    expect(document.title).toBe('(2) [dev] PDT');
    unmount();
    expect(document.title).toBe('PDT');
  });

  it('leaves the stable title alone', async () => {
    const calls = stubApi({ 'GET /api/health': health(false, 7420) });
    renderHook(() => useDevTitle(1), { wrapper: wrapper() });
    await waitFor(() => expect(calls).toHaveLength(1));
    await act(async () => {});
    expect(document.title).toBe('(1) PDT');
  });
});
