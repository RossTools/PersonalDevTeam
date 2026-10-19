import { act, render, screen, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider, useQueryClient } from '@tanstack/react-query';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { stubApi } from '../test-utils';
import { hasUnsavedInput, useReloadOnNewBuild } from './useReloadOnNewBuild';

afterEach(() => {
  vi.unstubAllGlobals();
  vi.useRealTimers();
  document.body.innerHTML = '';
});

const health = (webBuild: string | null) => ({ status: 'ok', name: 'PDT', version: '0.1.0', profile: 'dev', port: 8512, dev: true, webBuild });

function Probe({ reload }: { reload: () => void }) {
  useReloadOnNewBuild(reload);
  const data = useQueryClient().getQueryData<{ webBuild: string | null }>(['health']);
  return <p>build {String(data?.webBuild)}</p>;
}

/** Fetches health again and waits until the page has rendered (and so checked) that build. */
async function poll(qc: QueryClient, build: string | null) {
  await act(() => qc.refetchQueries({ queryKey: ['health'] }));
  await screen.findByText(`build ${build}`);
}

function mount(reload: () => void) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(<QueryClientProvider client={qc}><Probe reload={reload} /></QueryClientProvider>);
  return qc;
}

describe('useReloadOnNewBuild', () => {
  it('reloads once the web build differs from the one the page loaded against', async () => {
    let build: string | null = 'aaa';
    const calls = stubApi({ 'GET /api/health': () => health(build) });
    const reload = vi.fn();
    const qc = mount(reload);
    await poll(qc, 'aaa');
    expect(calls.length).toBeGreaterThan(0);
    expect(reload).not.toHaveBeenCalled();
    build = 'bbb';
    await poll(qc, 'bbb');
    await waitFor(() => expect(reload).toHaveBeenCalled());
  });

  it('never reloads without a web build (no web dir)', async () => {
    let build: string | null = null;
    stubApi({ 'GET /api/health': () => health(build) });
    const reload = vi.fn();
    const qc = mount(reload);
    await poll(qc, null);
    build = 'bbb';
    await poll(qc, 'bbb');
    expect(reload).not.toHaveBeenCalled();
  });

  it('holds the reload while a field has text, and reloads once it is cleared', async () => {
    let build = 'aaa';
    stubApi({ 'GET /api/health': () => health(build) });
    const input = document.createElement('input');
    document.body.append(input);
    const reload = vi.fn();
    const qc = mount(reload);
    await poll(qc, 'aaa');
    input.focus();
    input.value = 'half a ticket title';
    build = 'bbb';
    await poll(qc, 'bbb');
    expect(reload).not.toHaveBeenCalled();
    input.value = '';
    await act(() => qc.refetchQueries({ queryKey: ['health'] }));
    await waitFor(() => expect(reload).toHaveBeenCalled());
  });
});

describe('hasUnsavedInput', () => {
  it('counts an open modal and a focused field with text, not an empty field or the terminal', () => {
    expect(hasUnsavedInput()).toBe(false);
    const area = document.createElement('textarea');
    document.body.append(area);
    area.focus();
    expect(hasUnsavedInput()).toBe(false);
    area.value = 'draft';
    expect(hasUnsavedInput()).toBe(true);
    area.classList.add('xterm-helper-textarea');
    expect(hasUnsavedInput()).toBe(false);
    const modal = document.createElement('div');
    modal.setAttribute('aria-modal', 'true');
    document.body.append(modal);
    expect(hasUnsavedInput()).toBe(true);
  });
});
