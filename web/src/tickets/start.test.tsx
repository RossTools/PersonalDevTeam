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

/** Host settings with the given default agent and view (only the fields the dialog reads). */
const hostDefaults = (defaultAgent: string, defaultSessionMode: string) => ({ settings: { defaultAgent, defaultSessionMode } });

function stub(t: Ticket, start: Parameters<typeof stubApi>[0][string] = () => started(t), defaults?: object): Call[] {
  return stubApi({
    'GET /api/tickets/t1': t,
    'GET /api/projects': [{ ...project, baseBranch: 'trunk' }],
    'POST /api/tickets/t1/start': start,
    ...(defaults ? { 'GET /api/host/settings': defaults } : {}),
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
  it('starts with a check, the chosen agent, typed branch and base, and the tab placement', async () => {
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
      agent: 'codex', sessionMode: 'conversation', branch: 'feat/login', permissionMode: 'bypass', tab: { workspaceId: 'w1', position: 2 },
      mode: 'check', cols: 120, rows: 32,
    }]);
    expect(onClose).toHaveBeenCalled();
  });

  it("starts with the host's default agent and view", async () => {
    const calls = stub(ready, undefined, hostDefaults('codex', 'terminal'));
    open();
    const dialog = await screen.findByRole('dialog', { name: 'Start ALP-1' });
    await waitFor(() => expect(within(dialog).getByRole('radio', { name: 'Codex' })).toBeChecked());
    expect(within(dialog).getByRole('radio', { name: 'Terminal' })).toBeChecked();
    await userEvent.click(within(dialog).getByRole('button', { name: 'Start' }));
    await waitFor(() => expect(starts(calls)[0]).toMatchObject({ agent: 'codex', sessionMode: 'terminal' }));
  });

  it('can start Devin as a conversation or a terminal', async () => {
    const calls = stub(ready, undefined, hostDefaults('devin', 'conversation'));
    open();
    const dialog = await screen.findByRole('dialog', { name: 'Start ALP-1' });
    await waitFor(() => expect(within(dialog).getByRole('radio', { name: 'Devin' })).toBeChecked());
    expect(within(dialog).getByRole('radio', { name: 'Conversation' })).toBeChecked();
    await userEvent.click(within(dialog).getByRole('button', { name: 'Start' }));
    await waitFor(() => expect(starts(calls)[0]).toMatchObject({ agent: 'devin', sessionMode: 'conversation' }));
  });

  it('offers only the agents the host has, a logged-out one with a hint', async () => {
    const tool = (found: boolean, ok: boolean) => ({ found, ok, detail: '' });
    const calls = stubApi({
      'GET /api/tickets/t1': ready,
      'GET /api/projects': [project],
      'GET /api/host/checks': { claude: tool(true, false), codex: tool(true, true), devin: tool(false, false), git: tool(true, true), tailscale: tool(true, true), checkedAt: 1 },
      'GET /api/host/settings': hostDefaults('devin', 'conversation'),
      'POST /api/tickets/t1/start': () => started(ready),
    });
    open();
    const dialog = await screen.findByRole('dialog', { name: 'Start ALP-1' });
    await waitFor(() => expect(within(dialog).queryByRole('radio', { name: /Devin/ })).toBeNull());
    // The default Devin is missing: the first agent the host has instead.
    expect(within(dialog).getByRole('radio', { name: 'Claude · not logged in' })).toBeChecked();
    expect(within(dialog).getByRole('radio', { name: 'Codex' })).toBeInTheDocument();
    await userEvent.click(within(dialog).getByRole('button', { name: 'Start' }));
    await waitFor(() => expect(starts(calls)[0]).toMatchObject({ agent: 'claude' }));
  });

  it('starts a conversation by default, or the terminal when chosen', async () => {
    const calls = stub(ready);
    const { onStarted } = open();
    const dialog = await screen.findByRole('dialog', { name: 'Start ALP-1' });
    expect(within(dialog).getByRole('radio', { name: 'Conversation' })).toBeChecked();
    expect(within(dialog).getByText('The ticket is its first message.')).toBeInTheDocument();
    await userEvent.click(within(dialog).getByRole('radio', { name: 'Terminal' }));
    await userEvent.click(within(dialog).getByRole('button', { name: 'Start' }));
    await waitFor(() => expect(onStarted).toHaveBeenCalled());
    expect(starts(calls)[0]).toMatchObject({ sessionMode: 'terminal' });
  });

  it('can start without a session', async () => {
    const calls = stub(ready);
    const { onStarted } = open();
    const dialog = await screen.findByRole('dialog', { name: 'Start ALP-1' });
    await userEvent.click(within(dialog).getByRole('radio', { name: 'No session' }));
    expect(within(dialog).queryByRole('radio', { name: 'Claude' })).toBeNull();
    expect(within(dialog).queryByRole('checkbox', { name: /Skip permission prompts/ })).toBeNull();
    await userEvent.click(within(dialog).getByRole('button', { name: 'Start' }));
    await waitFor(() => expect(onStarted).toHaveBeenCalled());
    expect(starts(calls)[0]).toEqual({ agent: 'claude', noSession: true, newWorkspace: true, mode: 'check', cols: 120, rows: 32 });
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
    await userEvent.click(within(dialog).getByRole('button', { name: /Permission mode/ }));
    await userEvent.click(await screen.findByRole('option', { name: 'Ask' }));
    await userEvent.click(within(dialog).getByRole('button', { name: 'Start anyway' }));
    await waitFor(() => expect(starts(calls)).toHaveLength(1));
    expect(starts(calls)[0]).toMatchObject({ agent: 'claude', mode: 'force', permissionMode: 'ask' });
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

  it('defaults to bypassing permission prompts and a new workspace instead of the placement', async () => {
    const calls = stub(ready);
    open({ placement: { workspaceId: 'w1', position: 0 } });
    const dialog = await screen.findByRole('dialog', { name: 'Start ALP-1' });
    expect(within(dialog).getByRole('button', { name: /Permission mode/ })).toHaveTextContent('Bypass (dangerous)');
    expect(within(dialog).getByRole('switch', { name: 'Open in a new workspace' })).toBeChecked();
    // In the terminal it is a checkbox, on for Bypass.
    await userEvent.click(within(dialog).getByRole('radio', { name: 'Terminal' }));
    expect(within(dialog).getByRole('checkbox', { name: /Skip permission prompts/ })).toBeChecked();
    await userEvent.click(within(dialog).getByRole('button', { name: 'Start' }));
    await waitFor(() => expect(starts(calls)).toHaveLength(1));
    expect(starts(calls)[0]).toMatchObject({ newWorkspace: true, sessionMode: 'terminal', permissionMode: 'bypass' });
    expect(starts(calls)[0]).not.toHaveProperty('tab');
  });

  it("starts with the host's permission mode for tickets", async () => {
    const calls = stub(ready, undefined, { settings: { defaultAgent: 'claude', defaultSessionMode: 'conversation', ticketPermissionMode: 'accept_edits' } });
    open();
    const dialog = await screen.findByRole('dialog', { name: 'Start ALP-1' });
    await waitFor(() => expect(within(dialog).getByRole('button', { name: /Permission mode/ })).toHaveTextContent('Accept edits'));
    await userEvent.click(within(dialog).getByRole('button', { name: 'Start' }));
    await waitFor(() => expect(starts(calls)).toHaveLength(1));
    expect(starts(calls)[0]).toMatchObject({ permissionMode: 'accept_edits' });
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

  it('shows no branch or base for a Personal project and sends no branch', async () => {
    const calls = stubApi({
      'GET /api/tickets/t1': ready,
      'GET /api/projects': [{ ...project, kind: 'personal', baseBranch: '' }],
      'POST /api/tickets/t1/start': () => started(ready),
    });
    open();
    const dialog = await screen.findByRole('dialog', { name: 'Start ALP-1' });
    await within(dialog).findByText(/Personal project \(no repository\)/);
    expect(within(dialog).queryByRole('textbox', { name: 'Branch' })).not.toBeInTheDocument();
    expect(within(dialog).queryByRole('textbox', { name: 'Base branch' })).not.toBeInTheDocument();
    expect(within(dialog).getByRole('radio', { name: 'Claude' })).toBeInTheDocument();
    await userEvent.click(within(dialog).getByRole('button', { name: 'Start' }));
    await waitFor(() => expect(starts(calls)).toHaveLength(1));
    expect(starts(calls)[0]).not.toHaveProperty('branch');
    expect(starts(calls)[0]).not.toHaveProperty('base');
  });
});
