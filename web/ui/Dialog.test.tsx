import { useState } from 'react';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import { Dialog } from './Dialog';
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
});
