import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Session } from '../api/gen/Session';
import type { Ticket } from '../api/gen/Ticket';
import { renderWithQuery, Reply, stubApi, type Call } from '../test-utils';
import { project, ticket } from './fixtures';
import { StartDialog } from './StartDialog';

beforeEach(() => localStorage.clear());
afterEach(() => vi.unstubAllGlobals());

const ready = ticket(1, { title: 'Fix login', stage: 'ready', defaultBranch: 'pdt/ALP-1-fix-login' });
const session = { id: 's1', projectId: project.id } as Session;
const started = (t: Ticket) => ({ ticket: { ...t, stage: 'in_progress' }, attempt: { id: 'a1' }, session });

function stub(t: Ticket, start: Parameters<typeof stubApi>[0][string] = () => started(t)): Call[] {
  return stubApi({
    'GET /api/tickets/t1': t,
    'GET /api/projects': [{ ...project, baseBranch: 'trunk' }],
    'POST /api/tickets/t1/start': start,
  });
}

const starts = (calls: Call[]) => calls.filter((c) => c.method === 'POST' && c.path === '/api/tickets/t1/start').map((c) => c.body);

function open(props: Partial<Parameters<typeof StartDialog>[0]> = {}) {
  const onStarted = vi.fn();
  const onClose = vi.fn();
  renderWithQuery(<StartDialog ticketId="t1" onClose={onClose} onStarted={onStarted} {...props} />);
  return { onStarted, onClose };
}

describe('Start dialog', () => {
  it('starts with a check, the remembered agent, typed branch and base, and the tab placement', async () => {
    const calls = stub(ready);
    const { onStarted, onClose } = open({ placement: { workspaceId: 'w1', position: 2 } });
    const dialog = await screen.findByRole('dialog', { name: 'Start ALP-1' });
    const branch = within(dialog).getByRole('textbox', { name: 'Branch' });
    expect(branch).toHaveAttribute('placeholder', 'pdt/ALP-1-fix-login');
    await waitFor(() => expect(within(dialog).getByRole('textbox', { name: 'Base branch' })).toHaveAttribute('placeholder', 'trunk'));
    await userEvent.click(within(dialog).getByRole('radio', { name: 'Codex' }));
    await userEvent.type(branch, 'feat/login');
    // On by default; turned off here so the placement is used.
    await userEvent.click(within(dialog).getByRole('switch', { name: 'Open in a new workspace' }));
    await userEvent.click(within(dialog).getByRole('button', { name: 'Start' }));
    await waitFor(() => expect(onStarted).toHaveBeenCalled());
    expect(starts(calls)).toEqual([{
      agent: 'codex', branch: 'feat/login', bypassPermissions: true, tab: { workspaceId: 'w1', position: 2 }, mode: 'check', cols: 120, rows: 32,
    }]);
    expect(onClose).toHaveBeenCalled();
    expect(localStorage.getItem('pdt.tickets.startAgent')).toBe('codex');
  });

  it('remembers the last agent', async () => {
    localStorage.setItem('pdt.tickets.startAgent', 'codex');
    stub(ready);
    open();
    const dialog = await screen.findByRole('dialog', { name: 'Start ALP-1' });
    expect(within(dialog).getByRole('radio', { name: 'Codex' })).toBeChecked();
  });

  it('can start Devin, and remembers it', async () => {
    localStorage.setItem('pdt.tickets.startAgent', 'devin');
    const calls = stub(ready);
    open();
    const dialog = await screen.findByRole('dialog', { name: 'Start ALP-1' });
    expect(within(dialog).getByRole('radio', { name: 'Devin' })).toBeChecked();
    await userEvent.click(within(dialog).getByRole('button', { name: 'Start' }));
    await waitFor(() => expect(starts(calls)[0]).toMatchObject({ agent: 'devin' }));
  });

  it('lists open blockers and starts anyway with mode force', async () => {
    const blocked = {
      ...ready,
      links: [
        { kind: 'blocks' as const, direction: 'incoming' as const, id: 't2', key: 'ALP-2', title: 'Schema', stage: 'ready' as const },
        { kind: 'blocks' as const, direction: 'incoming' as const, id: 't3', key: 'ALP-3', title: 'Old', stage: 'done' as const },
      ],
    };
    const calls = stub(blocked);
    open();
    const dialog = await screen.findByRole('dialog', { name: 'Start ALP-1' });
    expect(within(dialog).getByText('Schema')).toBeInTheDocument();
    expect(within(dialog).queryByText('Old')).not.toBeInTheDocument();
    await userEvent.click(within(dialog).getByRole('checkbox', { name: /Skip permission prompts/ }));
    await userEvent.click(within(dialog).getByRole('button', { name: 'Start anyway' }));
    await waitFor(() => expect(starts(calls)).toHaveLength(1));
    expect(starts(calls)[0]).not.toHaveProperty('bypassPermissions');
    expect(starts(calls)[0]).toMatchObject({ agent: 'claude', mode: 'force' });
  });

  it('switches to Start anyway when the server answers start_blocked', async () => {
    let n = 0;
    const calls = stub(ready, () => (n++ === 0 ? new Reply(409, { code: 'start_blocked', message: 'Blocked by ALP-9' }) : started(ready)));
    const { onStarted } = open();
    const dialog = await screen.findByRole('dialog', { name: 'Start ALP-1' });
    await userEvent.click(within(dialog).getByRole('button', { name: 'Start' }));
    expect(await within(dialog).findByText('Blocked by ALP-9')).toBeInTheDocument();
    expect(onStarted).not.toHaveBeenCalled();
    await userEvent.click(within(dialog).getByRole('button', { name: 'Start anyway' }));
    await waitFor(() => expect(onStarted).toHaveBeenCalled());
    expect(starts(calls).map((b) => (b as { mode: string }).mode)).toEqual(['check', 'force']);
  });

  it('defaults to skipping permission prompts and a new workspace instead of the placement', async () => {
    const calls = stub(ready);
    open({ placement: { workspaceId: 'w1', position: 0 } });
    const dialog = await screen.findByRole('dialog', { name: 'Start ALP-1' });
    expect(within(dialog).getByRole('checkbox', { name: /Skip permission prompts/ })).toBeChecked();
    expect(within(dialog).getByRole('switch', { name: 'Open in a new workspace' })).toBeChecked();
    await userEvent.click(within(dialog).getByRole('button', { name: 'Start' }));
    await waitFor(() => expect(starts(calls)).toHaveLength(1));
    expect(starts(calls)[0]).toMatchObject({ newWorkspace: true, bypassPermissions: true });
    expect(starts(calls)[0]).not.toHaveProperty('tab');
  });

  it('shows other errors in the dialog and stays open', async () => {
    stub(ready, () => new Reply(400, { code: 'invalid_input', message: 'Branch name is not valid' }));
    const { onClose } = open();
    const dialog = await screen.findByRole('dialog', { name: 'Start ALP-1' });
    await userEvent.click(within(dialog).getByRole('button', { name: 'Start' }));
    expect(await within(dialog).findByRole('alert')).toHaveTextContent('Branch name is not valid');
    expect(within(dialog).getByRole('button', { name: 'Start' })).toBeInTheDocument();
    expect(onClose).not.toHaveBeenCalled();
  });

  it('does not offer Start for a ticket with an active attempt', async () => {
    stub({ ...ready, stage: 'in_progress', attempt: { id: 'a1', branch: 'b', baseBranch: 'main', worktreeId: null, state: 'active', createdAt: 1, endedAt: null, lastHead: null, workspaceId: null, pr: null } });
    open();
    const dialog = await screen.findByRole('dialog', { name: 'Start ALP-1' });
    expect(within(dialog).getByText(/already has an active attempt/)).toBeInTheDocument();
    expect(within(dialog).getByRole('button', { name: 'Start' })).toBeDisabled();
  });
});
