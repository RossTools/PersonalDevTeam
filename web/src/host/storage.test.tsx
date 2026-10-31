import { cleanup, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { HostSettingsView } from '../api/gen/HostSettingsView';
import type { StorageView } from '../api/gen/StorageView';
import { atRisk, formatBytes, sizeText } from '../projects/worktreeStorage';
import { renderWithQuery, stubApi } from '../test-utils';
import { Storage } from './StorageSettings';

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

const GB = 1024 ** 3;
const host = { settings: { lowDiskGb: 20, criticalDiskGb: 5 } } as unknown as HostSettingsView;
const size = (over: object = {}) => ({ total: 4 * GB, build: 3 * GB, uncommitted: false, unpushed: 0, neverPushed: false, measuredAt: 1, ...over });

const view = (over: Partial<StorageView> = {}): StorageView => ({
  disks: [{ folders: ['/data', '/work/app'], space: { free: 12 * GB, total: 500 * GB }, level: 'low' }],
  projects: [{
    projectId: 'p', name: 'App', key: 'APP',
    buildOutput: { paths: ['target', '**/node_modules'], detected: true, error: null },
    settings: { cleanAfterDays: 2, removeAfterDays: 0 },
    total: 4 * GB, build: 3 * GB,
    worktrees: [
      { id: 'w1', branch: 'pdt-APP-1-x', path: '/work/app/.worktrees/a', missing: false, size: size(),
        ticket: { id: 't1', key: 'APP-1', title: 'Finished work', stage: 'done', finished: true } },
      { id: 'w2', branch: 'pdt-APP-2-y', path: '/work/app/.worktrees/b', missing: false, size: null,
        ticket: { id: 't2', key: 'APP-2', title: 'Ongoing', stage: 'in_progress', finished: false } },
    ],
  }],
  lowDiskGb: 20, criticalDiskGb: 5, autoStartPaused: false, measuring: false,
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
      'POST /api/worktrees/w1/clean': { freed: 3 * GB, removed: ['target'], kept: [], errors: [] },
    });
    renderWithQuery(<Storage v={host} />);
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

  it('says when auto-start is paused for disk space', async () => {
    stubApi({ 'GET /api/storage': view({ autoStartPaused: true }) });
    renderWithQuery(<Storage v={host} />);
    expect(await screen.findByText(/auto-start starts no new tickets/)).toBeInTheDocument();
  });
});
