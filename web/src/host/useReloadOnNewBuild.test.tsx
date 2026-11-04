import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider, useQuery } from '@tanstack/react-query';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { stubApi } from '../test-utils';
import { hasUnsavedInput, servedBuild, useReloadOnNewBuild } from './useReloadOnNewBuild';
import { NewBuildBanner } from './NewBuildBanner';

afterEach(() => {
  vi.unstubAllGlobals();
  vi.useRealTimers();
  document.body.innerHTML = '';
  document.head.innerHTML = '';
});

const health = (webBuild: string | null) => ({ status: 'ok', name: 'PDT', version: '0.1.0', profile: 'dev', port: 8512, dev: true, webBuild });

function Probe({ reload }: { reload: () => void }) {
  useReloadOnNewBuild(reload);
  return <Shown />;
}

/** The health answer the page has now (observing the query without fetching it). */
function Shown() {
  const { data } = useQuery<{ webBuild: string | null }>({ queryKey: ['health'], enabled: false });
  return <p>build {String(data?.webBuild)}</p>;
}

/** Fetches health again and waits until the page has rendered (and so checked) that build. */
async function poll(qc: QueryClient, build: string | null) {
  await act(() => qc.refetchQueries({ queryKey: ['health'] }));
  await screen.findByText(`build ${build}`);
}

function mount(reload: () => void, banner = false) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const page = banner ? <><NewBuildBanner reload={reload} /><Shown /></> : <Probe reload={reload} />;
  render(<QueryClientProvider client={qc}>{page}</QueryClientProvider>);
  return qc;
}

/** The meta a production build writes into index.html. */
function serve(build: string) {
  const meta = document.createElement('meta');
  meta.name = 'web-build';
  meta.content = build;
  document.head.append(meta);
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

describe('useReloadOnNewBuild against the build the page was served', () => {
  it('reloads when the first answer is already a newer build (a tab restored on a phone)', async () => {
    serve('aaa');
    expect(servedBuild()).toBe('aaa');
    stubApi({ 'GET /api/health': () => health('bbb') });
    const reload = vi.fn();
    mount(reload);
    await waitFor(() => expect(reload).toHaveBeenCalled());
  });

  it('stays while health reports the build it was served', async () => {
    serve('aaa');
    stubApi({ 'GET /api/health': () => health('aaa') });
    const reload = vi.fn();
    const qc = mount(reload);
    await poll(qc, 'aaa');
    await poll(qc, 'aaa');
    expect(reload).not.toHaveBeenCalled();
  });

  it('asks again at once when the page comes back: visible, focus, online, or restored from the back/forward cache', async () => {
    const calls = stubApi({ 'GET /api/health': () => health('aaa') });
    const qc = mount(vi.fn());
    await poll(qc, 'aaa');
    const asked = () => calls.filter((c) => c.path === '/api/health').length;
    const resume = async (fire: () => void) => {
      const before = asked();
      fire();
      await waitFor(() => expect(asked()).toBe(before + 1));
    };
    await resume(() => document.dispatchEvent(new Event('visibilitychange')));
    await resume(() => window.dispatchEvent(new Event('focus')));
    await resume(() => window.dispatchEvent(new Event('online')));
    await resume(() => window.dispatchEvent(Object.assign(new Event('pageshow'), { persisted: true })));
    // A first load's pageshow isn't a resume.
    const before = asked();
    window.dispatchEvent(Object.assign(new Event('pageshow'), { persisted: false }));
    await act(() => new Promise((r) => setTimeout(r, 50)));
    expect(asked()).toBe(before);
  });

  it('offers Reload while a draft holds it back, and reloads by itself once the draft is cleared', async () => {
    serve('aaa');
    stubApi({ 'GET /api/health': () => health('bbb') });
    const area = document.createElement('textarea');
    document.body.append(area);
    area.focus();
    area.value = 'a message half written';
    const reload = vi.fn();
    const qc = mount(reload, true);
    await screen.findByText('PDT was updated');
    expect(reload).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole('button', { name: 'Reload' }));
    expect(reload).toHaveBeenCalledTimes(1);
    area.value = '';
    await act(() => qc.refetchQueries({ queryKey: ['health'] }));
    await waitFor(() => expect(reload).toHaveBeenCalledTimes(2));
  });

  it('shows no banner when nothing holds the reload back', async () => {
    stubApi({ 'GET /api/health': () => health('aaa') });
    const qc = mount(vi.fn(), true);
    await poll(qc, 'aaa');
    expect(screen.queryByText('PDT was updated')).toBeNull();
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
