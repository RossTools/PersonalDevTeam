import { cleanup, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { BuiltinSettingsView } from '../api/gen/BuiltinSettingsView';
import type { StorageView } from '../api/gen/StorageView';
import { atRisk, formatBytes, sizeText } from '../projects/worktreeStorage';
import { renderWithQuery, stubApi } from '../test-utils';
import { BuiltinSettingsBanner } from '../workflows/BuiltinSettingsBanner';
import { Storage } from './StorageSettings';

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

const GB = 1024 ** 3;
const defaults = { lowGb: 20, criticalGb: 5, pausePickup: true, cleanNow: true };
const cleanupView = (over: Partial<BuiltinSettingsView> = {}, values: Record<string, unknown> = {}): BuiltinSettingsView => ({
  workflowId: 'w9', name: 'Clean up storage', enabled: true, customized: false, params: [],
  values: { ...defaults, ...values }, defaults, channels: [], ...over,
});
const size = (over: object = {}) => ({ total: 4 * GB, build: 3 * GB, uncommitted: false, unpushed: 0, neverPushed: false, measuredAt: 1, ...over });

const view = (over: Partial<StorageView> = {}): StorageView => ({
  disks: [{ folders: ['/data', '/work/app'], space: { free: 12 * GB, total: 500 * GB }, level: 'low' }],
  projects: [{
    projectId: 'p', name: 'App', key: 'APP',
    buildOutput: { paths: ['target', '**/node_modules'], detected: true, error: null },
    settings: { cleanAfterDays: 2, removeAfterDays: 0, wontDo: 'now', wontDoAfterDays: 7 },
    total: 4 * GB, build: 3 * GB,
    worktrees: [
      { id: 'w1', branch: 'pdt-APP-1-x', path: '/work/app/.worktrees/a', missing: false, size: size(),
        ticket: { id: 't1', key: 'APP-1', title: 'Finished work', stage: 'done', finished: true } },
      { id: 'w2', branch: 'pdt-APP-2-y', path: '/work/app/.worktrees/b', missing: false, size: null,
        ticket: { id: 't2', key: 'APP-2', title: 'Ongoing', stage: 'in_progress', finished: false } },
    ],
  }],
  lowDiskGb: 20, criticalDiskGb: 5, autoStartPaused: false, pickupPause: null, measuring: false,
  ...over,
});

describe('worktree sizes', () => {
  it('says sizes and what would be lost for people', () => {
    expect(formatBytes(512)).toBe('512 bytes');
    expect(formatBytes(1.5 * GB)).toBe('1.5 GB');
    expect(formatBytes(23 * GB)).toBe('23 GB');
    expect(sizeText(size())).toBe('4.0 GB, 3.0 GB of it build output');
    expect(sizeText(null)).toBe('Not measured yet');
    expect(atRisk(size())).toBeNull();
    expect(atRisk(size({ uncommitted: true, unpushed: 2 }))).toBe("It has uncommitted changes and 2 commits its remote doesn't have.");
  });
});

describe('Settings › Storage', () => {
  it('shows free space, each worktree with its ticket and size, and cleans one', async () => {
    const calls = stubApi({
      'GET /api/storage': view(),
      'GET /api/storage/cleanup': cleanupView(),
      'POST /api/worktrees/w1/clean': { freed: 3 * GB, removed: ['target'], kept: [], errors: [] },
    });
    renderWithQuery(<Storage />);
    expect(await screen.findByText('12 GB free of 500 GB')).toBeInTheDocument();
    expect(screen.getByText('Low')).toBeInTheDocument();
    expect(screen.getByText('target, **/node_modules')).toBeInTheDocument();
    const rows = screen.getAllByRole('listitem');
    expect(rows[0]).toHaveTextContent('APP-1');
    expect(rows[0]).toHaveTextContent('Done');
    expect(rows[0]).toHaveTextContent('4.0 GB, 3.0 GB of it build output');
    expect(rows[1]).toHaveTextContent('Not measured yet');
    // Remove is offered for finished tickets' worktrees only.
    expect(within(rows[0]).getByRole('button', { name: /Remove/ })).toBeInTheDocument();
    expect(within(rows[1]).queryByRole('button', { name: /Remove/ })).toBeNull();
    await userEvent.click(within(rows[0]).getByRole('button', { name: /Clean/ }));
    await waitFor(() => expect(calls.some((c) => c.method === 'POST' && c.path === '/api/worktrees/w1/clean')).toBe(true));
  });

  it('says why pickup is paused, and resumes it', async () => {
    const pickupPause = { reason: 'the disk is almost full (3.0 GB free)', workflowId: 'w9', at: 1 };
    const calls = stubApi({
      'GET /api/storage': view({ autoStartPaused: true, pickupPause }),
      'GET /api/storage/cleanup': cleanupView(),
      'DELETE /api/pickup/pause': null,
    });
    renderWithQuery(<Storage />);
    expect(await screen.findByText(/Pickup is paused: the disk is almost full \(3.0 GB free\)/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Resume pickup' }));
    await waitFor(() => expect(calls.some((c) => c.method === 'DELETE' && c.path === '/api/pickup/pause')).toBe(true));
  });
});

describe('Settings › Storage › Cleanup (the Clean up storage workflow)', () => {
  const puts = (calls: { method: string; body: unknown }[]) => calls.filter((c) => c.method === 'PUT').map((c) => c.body);

  it("changes the workflow's inputs: the thresholds and what happens when a disk is almost full", async () => {
    let values: Record<string, unknown> = {};
    const calls = stubApi({
      'GET /api/storage': view(),
      'GET /api/storage/cleanup': () => cleanupView({}, values),
      'PUT /api/storage/cleanup': (c) => {
        values = { ...values, ...(c.body as { values: Record<string, unknown> }).values };
        return cleanupView({}, values);
      },
    });
    renderWithQuery(<Storage />);
    const low = await screen.findByRole('textbox', { name: 'Low-disk warning (GB)' });
    expect(low).toHaveValue('20');
    await userEvent.clear(low);
    await userEvent.type(low, '50');
    await userEvent.tab();
    await waitFor(() => expect(puts(calls)).toEqual([{ values: { lowGb: 50 } }]));
    await userEvent.click(screen.getByRole('switch', { name: 'Pause pickup' }));
    await waitFor(() => expect(puts(calls).at(-1)).toEqual({ values: { pausePickup: false } }));
    await userEvent.click(screen.getByRole('switch', { name: 'Clean finished worktrees now' }));
    await waitFor(() => expect(puts(calls).at(-1)).toEqual({ values: { cleanNow: false } }));
    expect(screen.getByRole('link', { name: 'Customize as a workflow →' })).toBeInTheDocument();
  });

  it('refuses a threshold that is not a whole number of GB', async () => {
    const calls = stubApi({ 'GET /api/storage': view(), 'GET /api/storage/cleanup': cleanupView() });
    renderWithQuery(<Storage />);
    const critical = await screen.findByRole('textbox', { name: 'Almost full below (GB)' });
    await userEvent.clear(critical);
    await userEvent.type(critical, '2.5');
    await userEvent.tab();
    expect(await screen.findByText(/a whole number of GB/)).toBeInTheDocument();
    expect(puts(calls)).toEqual([]);
  });

  it('names a customized workflow, with Open and Reset to default', async () => {
    let customized = true;
    const calls = stubApi({
      'GET /api/storage': view(),
      'GET /api/storage/cleanup': () => cleanupView({ customized }),
      'POST /api/storage/cleanup/reset': () => {
        customized = false;
        return cleanupView();
      },
    });
    renderWithQuery(<Storage />);
    expect(await screen.findByText('Cleanup follows your customized workflow Clean up storage.')).toBeInTheDocument();
    expect(screen.queryByRole('switch', { name: 'Pause pickup' })).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Open' })).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Reset to default' }));
    await waitFor(() => expect(calls.some((c) => c.method === 'POST' && c.path === '/api/storage/cleanup/reset')).toBe(true));
    expect(await screen.findByRole('switch', { name: 'Pause pickup' })).toBeInTheDocument();
  });

  it('points the workflow editor at Settings', () => {
    renderWithQuery(<BuiltinSettingsBanner builtin="clean-up-storage" />);
    expect(screen.getByText(/Its settings are in Settings → This host → Storage/)).toBeInTheDocument();
  });
});
