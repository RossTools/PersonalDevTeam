// A slow connection (PDT-575): until a list's first answer comes, a screen shows a placeholder, never its empty or
// first-run state.
import { act, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { renderWithQuery } from '../test-utils';
import { AppShell } from './AppShell';
import { PmList } from '../pm/PmList';

vi.mock('../terminal/TerminalView', () => ({ TerminalView: () => null }));

afterEach(() => vi.unstubAllGlobals());

/** Every call waits forever: nothing has loaded yet. */
function stubPending() {
  vi.stubGlobal('fetch', vi.fn(() => new Promise(() => {})));
}

async function openAt(path: string) {
  stubPending();
  window.history.replaceState(null, '', path);
  renderWithQuery(<AppShell />);
  // Let the first render's effects and lazy screens settle.
  await act(async () => { await new Promise((r) => setTimeout(r, 50)); });
}

describe('screens while their lists load (PDT-575)', () => {
  it('Sessions: no "No projects yet", "No agents yet", "No workspace" or "Add a project"', async () => {
    await openAt('/sessions');
    expect(screen.queryByText(/No projects yet/)).toBeNull();
    expect(screen.queryByText(/No agents yet/)).toBeNull();
    expect(screen.queryByText('No workspace')).toBeNull();
    expect(screen.queryByText(/Add a project to get started/)).toBeNull();
    expect(screen.getByRole('status', { name: 'Loading agents' })).toBeInTheDocument();
  });

  it('Inbox: not "Nothing waits on you"', async () => {
    await openAt('/inbox');
    expect(await screen.findByRole('status', { name: 'Loading the Inbox' }, { timeout: 5000 })).toBeInTheDocument();
    expect(screen.queryByText(/Nothing waits on you/)).toBeNull();
  });

  it('Tickets: not "Add a project in Settings"', async () => {
    await openAt('/tickets');
    expect(await screen.findByRole('status', { name: 'Loading tickets' }, { timeout: 5000 })).toBeInTheDocument();
    expect(screen.queryByText(/Add a project in Settings/)).toBeNull();
  });

  it('PM: no lone "New conversation" row', async () => {
    stubPending();
    renderWithQuery(<PmList selection="current" onSelect={() => {}} onNew={() => {}} />);
    expect(screen.getByRole('status', { name: 'Loading the PM' })).toBeInTheDocument();
    expect(screen.queryByText('Ask something to start one')).toBeNull();
  });
});
