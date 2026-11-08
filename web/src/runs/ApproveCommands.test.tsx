import { QueryClient } from '@tanstack/react-query';
import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ApiError } from '../api/client';
import type { ConfigCommand } from '../api/gen/ConfigCommand';
import { renderWithQuery, stubApi } from '../test-utils';
import { ConfigCommands, withApproval } from './ApproveCommands';
import { runKeys } from './queries';

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

const command = (fingerprint: string, over: Partial<ConfigCommand> = {}): ConfigCommand =>
  ({ fingerprint, uses: [`run "${fingerprint}"`], setup: null, command: `echo ${fingerprint}`, env: {}, ...over });

const untrusted = (commands: ConfigCommand[], projectId = 'p') => new ApiError(409, 'untrusted_commands', 'no', { projectId, commands });

describe('withApproval', () => {
  it('runs once and asks nothing when no command waits', async () => {
    const confirm = vi.fn();
    await expect(withApproval(confirm, new QueryClient(), async () => 'ran')).resolves.toBe('ran');
    expect(confirm).not.toHaveBeenCalled();
  });

  it('passes on any other error, and an untrusted_commands error that names no commands', async () => {
    const confirm = vi.fn();
    const qc = new QueryClient();
    const other = new ApiError(500, 'internal', 'boom');
    await expect(withApproval(confirm, qc, () => Promise.reject(other))).rejects.toBe(other);
    const empty = untrusted([]);
    await expect(withApproval(confirm, qc, () => Promise.reject(empty))).rejects.toBe(empty);
    await expect(withApproval(confirm, qc, () => Promise.reject(new Error('plain')))).rejects.toThrow('plain');
    expect(confirm).not.toHaveBeenCalled();
  });

  it('asks about several commands at once, shows each as it runs, and approves them all before trying again', async () => {
    const calls = stubApi({ 'POST /api/projects/p/commands/trust': undefined });
    const confirm = vi.fn().mockResolvedValue({ ok: true });
    const attempt = vi.fn().mockRejectedValueOnce(untrusted([command('a', { setup: 'pnpm i', env: { PORT: '1' } }), command('b')])).mockResolvedValue('ran');
    await expect(withApproval(confirm, new QueryClient(), attempt)).resolves.toBe('ran');
    const asked = confirm.mock.calls[0][0];
    expect(asked.title).toBe('Run these commands?');
    expect(asked.action).toBe('Approve and run');
    render(<>{asked.body}</>);
    expect(screen.getByText(/They come from the project's run configuration and PDT hasn't run them before/)).toBeInTheDocument();
    expect(screen.getByRole('group', { name: 'Command for run "a"' })).toHaveTextContent('pnpm i PORT=1 echo a');
    expect(screen.getByRole('group', { name: 'Command for run "b"' })).toHaveTextContent('echo b');
    expect(calls.find((c) => c.path === '/api/projects/p/commands/trust')?.body).toEqual({ fingerprints: ['a', 'b'] });
    expect(attempt).toHaveBeenCalledTimes(2);
  });

  it('declined: resolves to null, trusts nothing and tries nothing again', async () => {
    const calls = stubApi({});
    const attempt = vi.fn().mockRejectedValue(untrusted([command('a')]));
    const confirm = vi.fn().mockResolvedValue({ ok: false });
    await expect(withApproval(confirm, new QueryClient(), attempt)).resolves.toBeNull();
    expect(confirm.mock.calls[0][0].title).toBe('Run this command?');
    expect(attempt).toHaveBeenCalledTimes(1);
    expect(calls).toEqual([]);
  });
});

describe('ConfigCommands', () => {
  it('lists the commands waiting for approval first; Approve trusts one', async () => {
    let trusted: string[] = [];
    const calls = stubApi({
      'GET /api/projects/p/commands': () => ({ commands: [command('a'), command('b'), command('c')], untrusted: ['b', 'c'].filter((f) => !trusted.includes(f)), error: null }),
      'POST /api/projects/p/commands/trust': (c: { body: { fingerprints: string[] } }) => {
        trusted = [...trusted, ...c.body.fingerprints];
      },
    });
    renderWithQuery(<ConfigCommands projectId="p" />);
    await screen.findByRole('group', { name: 'Command for run "a"' });
    const names = () => screen.getAllByRole('group').map((g) => g.getAttribute('aria-label'));
    expect(names()).toEqual(['Command for run "b"', 'Command for run "c"', 'Command for run "a"']);
    expect(screen.getByRole('group', { name: 'Command for run "a"' })).toHaveTextContent('Approved');
    const b = screen.getByRole('group', { name: 'Command for run "b"' });
    expect(b).toHaveTextContent('Not approved');
    await userEvent.click(within(b).getByRole('button', { name: 'Approve' }));
    await waitFor(() => expect(screen.getByRole('group', { name: 'Command for run "b"' })).not.toHaveTextContent('Not approved'));
    expect(calls.find((c) => c.method === 'POST')?.body).toEqual({ fingerprints: ['b'] });
    expect(screen.getByRole('group', { name: 'Command for run "c"' })).toHaveTextContent('Not approved');
  });

  it('says when the configuration names no commands, and shows nothing when it is invalid', async () => {
    stubApi({ 'GET /api/projects/p/commands': { commands: [], untrusted: [], error: null } });
    const { unmount } = renderWithQuery(<ConfigCommands projectId="p" />);
    expect(await screen.findByText('The run configuration names no commands.')).toBeInTheDocument();
    unmount();
    stubApi({ 'GET /api/projects/p/commands': { commands: [command('a')], untrusted: [], error: 'bad toml' } });
    const { qc } = renderWithQuery(<ConfigCommands projectId="p" />);
    await waitFor(() => expect(qc.getQueryState(runKeys.commands('p'))?.status).toBe('success'));
    expect(screen.queryByRole('group')).toBeNull();
    expect(screen.queryByText(/Loading the commands|names no commands/)).toBeNull();
  });
});
