import { useState } from 'react';
import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { CommandPalette } from './CommandPalette';
import type { PaletteItem } from './paletteItems';

function setup() {
  const fns = { scratch: vi.fn(), close: vi.fn(), moveTo: vi.fn(), moveNew: vi.fn() };
  const items: PaletteItem[] = [
    { id: 'workspace:w2', label: 'Workspace · Scratch', group: 'Go to', icon: 'workspace', run: fns.scratch },
    {
      id: 'tab:t1', label: 'fix login', detail: 'PersonalDevTeam', group: 'Go to', keywords: ['PDT', 'fix-login'],
      state: { dot: 'waiting', pulse: false, label: 'Needs you', tone: 'waiting', unseen: false }, run: vi.fn(),
    },
    { id: 'tab:close', label: 'Close tab', group: 'Actions', icon: 'close', shortcut: 'Ctrl+] x', run: fns.close },
    {
      id: 'tab:move', label: 'Move tab to workspace…', group: 'Actions', icon: 'move',
      next: {
        kind: 'list', title: 'Move tab to workspace', placeholder: 'Filter workspaces…',
        items: [
          { id: 'move:w2', label: 'Scratch', group: 'Workspaces', run: () => fns.moveTo('w2') },
          {
            id: 'move:new', label: 'New workspace…', group: 'Workspaces',
            next: { kind: 'input', title: 'New workspace', label: 'Name', submitLabel: 'Create and move', run: fns.moveNew },
          },
        ],
      },
    },
  ];
  function Harness() {
    const [open, setOpen] = useState(false);
    return (
      <>
        <button onClick={() => setOpen(true)}>Open palette</button>
        <CommandPalette open={open} onOpenChange={setOpen} items={items} />
      </>
    );
  }
  render(<Harness />);
  return fns;
}

const user = () => userEvent.setup();

async function openPalette(u: ReturnType<typeof userEvent.setup>) {
  await u.click(screen.getByRole('button', { name: 'Open palette' }));
  return screen.findByRole('dialog', { name: 'Command palette' });
}

describe('CommandPalette', () => {
  it('opens with the filter focused, grouped items, key hints, details and state dots', async () => {
    const u = user();
    setup();
    const dialog = await openPalette(u);
    expect(within(dialog).getByRole('searchbox')).toHaveFocus();
    expect(within(dialog).getByText('Go to')).toBeInTheDocument();
    expect(within(dialog).getByText('Actions')).toBeInTheDocument();
    expect(within(dialog).getByText('Ctrl+] x').tagName).toBe('KBD');
    expect(within(dialog).getByRole('menuitem', { name: /fix login PersonalDevTeam/ })).toBeInTheDocument();
    expect(within(dialog).getByLabelText('Needs you')).toBeInTheDocument();
    await u.keyboard('fix-login');
    expect(within(dialog).getAllByRole('menuitem').map((e) => e.textContent)).toEqual(['fix login PersonalDevTeam']);
  });

  it('filters; Enter runs the highlighted item, closes and returns focus', async () => {
    const u = user();
    const { close } = setup();
    await openPalette(u);
    await u.keyboard('close');
    expect(screen.getAllByRole('menuitem').map((e) => e.textContent)).toEqual(['Close tabCtrl+] x']);
    await u.keyboard('{ArrowDown}{Enter}');
    expect(close).toHaveBeenCalledTimes(1);
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    await waitFor(() => expect(screen.getByRole('button', { name: 'Open palette' })).toHaveFocus());
  });

  it('runs an item on click', async () => {
    const u = user();
    const { scratch } = setup();
    await openPalette(u);
    await u.click(screen.getByRole('menuitem', { name: /Workspace · Scratch/ }));
    expect(scratch).toHaveBeenCalled();
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  });

  it('moves a tab in two steps; Escape goes back a step, then closes', async () => {
    const u = user();
    const { moveTo } = setup();
    await openPalette(u);
    await u.click(screen.getByRole('menuitem', { name: /Move tab to workspace/ }));
    const step = await screen.findByRole('dialog', { name: 'Move tab to workspace' });
    expect(within(step).getByRole('searchbox')).toHaveFocus();
    expect(within(step).getAllByRole('menuitem').map((e) => e.textContent)).toEqual(['Scratch', 'New workspace…']);
    await u.keyboard('{Escape}');
    expect(await screen.findByRole('dialog', { name: 'Command palette' })).toBeInTheDocument();
    await u.click(screen.getByRole('menuitem', { name: /Move tab to workspace/ }));
    await u.click(await screen.findByRole('menuitem', { name: 'Scratch' }));
    expect(moveTo).toHaveBeenCalledWith('w2');
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  });

  it('names a new workspace in a third step', async () => {
    const u = user();
    const { moveNew } = setup();
    await openPalette(u);
    await u.click(screen.getByRole('menuitem', { name: /Move tab to workspace/ }));
    await u.click(await screen.findByRole('menuitem', { name: 'New workspace…' }));
    const name = await screen.findByRole('textbox', { name: 'Name' });
    await waitFor(() => expect(name).toHaveFocus());
    expect(screen.getByRole('button', { name: 'Create and move' })).toBeDisabled();
    await u.keyboard('Release{Enter}');
    expect(moveNew).toHaveBeenCalledWith('Release');
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  });

  it('closes on Escape at the first step and starts over when opened again', async () => {
    const u = user();
    setup();
    await openPalette(u);
    await u.click(screen.getByRole('menuitem', { name: /Move tab to workspace/ }));
    await screen.findByRole('dialog', { name: 'Move tab to workspace' });
    await u.keyboard('{Escape}{Escape}');
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(await openPalette(u)).toBeInTheDocument();
  });
});
