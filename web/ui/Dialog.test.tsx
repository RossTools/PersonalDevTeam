import { useState } from 'react';
import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import { Dialog, DialogActions } from './Dialog';
import { Button } from './Button';

function Controlled() {
  const [open, setOpen] = useState(false);
  return (
    <>
      <button onClick={() => setOpen(true)}>Add project</button>
      <Dialog
        open={open}
        onOpenChange={setOpen}
        title="Add project"
        description="A git repository on this machine."
        footer={<button onClick={() => setOpen(false)}>Done</button>}
      >
        <input aria-label="Folder" />
      </Dialog>
    </>
  );
}

describe('Dialog', () => {
  it('opens as a labelled modal with description and footer', async () => {
    render(<Controlled />);
    expect(screen.queryByRole('dialog')).toBeNull();
    await userEvent.click(screen.getByRole('button', { name: 'Add project' }));
    const d = await screen.findByRole('dialog', { name: 'Add project' });
    expect(d).toHaveAccessibleDescription('A git repository on this machine.');
    expect(screen.getByRole('button', { name: 'Done' })).toBeInTheDocument();
  });

  it('Escape closes and focus returns to the opener', async () => {
    render(<Controlled />);
    const opener = screen.getByRole('button', { name: 'Add project' });
    await userEvent.click(opener);
    await screen.findByRole('dialog');
    await userEvent.keyboard('{Escape}');
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    await waitFor(() => expect(opener).toHaveFocus());
  });

  it('traps focus inside', async () => {
    render(<Controlled />);
    await userEvent.click(screen.getByRole('button', { name: 'Add project' }));
    const d = await screen.findByRole('dialog');
    for (let i = 0; i < 5; i++) {
      await userEvent.tab();
      expect(d.contains(document.activeElement)).toBe(true);
    }
  });

  it('the close button closes it', async () => {
    render(<Controlled />);
    await userEvent.click(screen.getByRole('button', { name: 'Add project' }));
    await screen.findByRole('dialog');
    await userEvent.click(screen.getByRole('button', { name: 'Close' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  });

  it('the backdrop closes it', async () => {
    render(<Controlled />);
    await userEvent.click(screen.getByRole('button', { name: 'Add project' }));
    await screen.findByRole('dialog');
    const overlay = document.querySelector('[data-slot="dialog-overlay"]')!;
    expect(overlay.className).toContain('bg-backdrop');
    await userEvent.click(overlay);
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  });

  it('opens from a trigger', async () => {
    render(
      <Dialog trigger={<Button>Shortcuts</Button>} title="Keyboard shortcuts">
        <p>Press Ctrl+] then one key.</p>
      </Dialog>,
    );
    await userEvent.click(screen.getByRole('button', { name: 'Shortcuts' }));
    expect(await screen.findByRole('dialog', { name: 'Keyboard shortcuts' })).toBeInTheDocument();
    await userEvent.keyboard('{Escape}');
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    await waitFor(() => expect(screen.getByRole('button', { name: 'Shortcuts' })).toHaveFocus());
  });

  it('puts header actions beside the title', async () => {
    render(<Dialog open title="New workflow" headerActions={<input aria-label="Search" />}>body</Dialog>);
    const d = await screen.findByRole('dialog', { name: 'New workflow' });
    expect(within(d).getByRole('textbox', { name: 'Search' })).toBeInTheDocument();
  });

  it('with onClose, is open and calls onClose when it closes', async () => {
    const closed: string[] = [];
    render(<Dialog onClose={() => closed.push('x')} title="Rename">body</Dialog>);
    await screen.findByRole('dialog', { name: 'Rename' });
    await userEvent.keyboard('{Escape}');
    expect(closed).toEqual(['x']);
  });

  it('DialogActions: Cancel, then the named action', async () => {
    const calls: string[] = [];
    render(
      <Dialog
        onClose={() => calls.push('close')}
        title="New document"
        footer={<DialogActions onCancel={() => calls.push('cancel')} action="Create" onAction={() => calls.push('create')} />}
      >
        body
      </Dialog>,
    );
    const d = await screen.findByRole('dialog');
    const buttons = within(d).getAllByRole('button').map((b) => b.textContent);
    expect(buttons.indexOf('Cancel')).toBe(buttons.indexOf('Create') - 1);
    await userEvent.click(within(d).getByRole('button', { name: 'Create' }));
    await userEvent.click(within(d).getByRole('button', { name: 'Cancel' }));
    expect(calls).toEqual(['create', 'cancel']);
  });

  it('DialogActions with a form submits it', async () => {
    const submitted: string[] = [];
    render(
      <Dialog open title="Rename" footer={<DialogActions onCancel={() => {}} action="Rename" form="rename" danger />}>
        <form id="rename" onSubmit={(e) => { e.preventDefault(); submitted.push('yes'); }} />
      </Dialog>,
    );
    await userEvent.click(await screen.findByRole('button', { name: 'Rename' }));
    expect(submitted).toEqual(['yes']);
  });
});
