import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { useState } from 'react';
import { Dialog } from './Dialog';
import { Sheet } from './Sheet';
import { Button } from './primitives/button';

describe('Sheet', () => {
  it('opens from a button, is named by its title, closes on Escape and returns focus', async () => {
    const user = userEvent.setup();
    render(
      <Sheet side="left" title="Workspaces" trigger={<Button>Alpha</Button>}>
        <a href="#x">Release</a>
      </Sheet>,
    );
    // The sheet loads lazily; until then its trigger is shown as a plain placeholder.
    await waitFor(() => expect(screen.getByRole('button', { name: 'Alpha' })).toHaveAttribute('aria-expanded', 'false'));
    const trigger = screen.getByRole('button', { name: 'Alpha' });
    await user.click(trigger);
    const sheet = await screen.findByRole('dialog', { name: 'Workspaces' });
    expect(within(sheet).getByRole('link', { name: 'Release' })).toBeInTheDocument();
    await user.keyboard('{Escape}');
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    await waitFor(() => expect(trigger).toHaveFocus());
  });

  it('closes with its close button', async () => {
    const user = userEvent.setup();
    const onOpenChange = vi.fn();
    render(
      <Sheet side="bottom" title="Peek" open onOpenChange={onOpenChange}>
        body
      </Sheet>,
    );
    const sheet = await screen.findByRole('dialog', { name: 'Peek' });
    await user.click(within(sheet).getByRole('button', { name: 'Close' }));
    expect(onOpenChange).toHaveBeenCalledWith(false);
  });

  it('closes when modal on a press on its backdrop only, not on one that lands on the page around it', async () => {
    const user = userEvent.setup();
    const onOpenChange = vi.fn();
    render(<Sheet side="left" title="Sidebar" open onOpenChange={onOpenChange}>body</Sheet>);
    await screen.findByRole('dialog', { name: 'Sidebar' });
    // Where a click lands when a menu opened on mouse down covers its button by mouse up.
    await user.click(document.body);
    expect(onOpenChange).not.toHaveBeenCalled();
    await user.click(document.querySelector('[data-slot="drawer-viewport"]')!);
    expect(onOpenChange).toHaveBeenCalledWith(false);
  });

  it('when not modal, closes on a click on the page beside it, not on one in a portal or on what keeps it open', async () => {
    const user = userEvent.setup();
    const onOpenChange = vi.fn();
    render(
      <>
        <button type="button">Beside</button>
        <button type="button" data-row>Row</button>
        <Sheet side="right" title="Peek" modal={false} open onOpenChange={onOpenChange} keepOpenOn={(t) => !!t.closest('[data-row]')}>
          body
        </Sheet>
      </>,
    );
    const sheet = await screen.findByRole('dialog', { name: 'Peek' });
    // The sheet takes focus once it opens; moving focus out of it must not close it.
    await waitFor(() => expect(sheet.contains(document.activeElement)).toBe(true));
    await user.click(within(sheet).getByText('body'));
    // A menu or dialog the sheet opened renders in a portal of its own.
    const portal = document.body.appendChild(document.createElement('div'));
    portal.innerHTML = '<button type="button">In a menu</button>';
    await user.click(screen.getByRole('button', { name: 'In a menu' }));
    portal.remove();
    await user.click(screen.getByRole('button', { name: 'Row' }));
    expect(screen.getByRole('button', { name: 'Row' })).toHaveFocus();
    expect(onOpenChange).not.toHaveBeenCalled();
    await user.click(screen.getByRole('button', { name: 'Beside' }));
    expect(onOpenChange).toHaveBeenCalledWith(false);
  });

  it('opened from a dialog, is usable, keeps Tab inside and closes on Escape without closing the dialog', async () => {
    const user = userEvent.setup();
    const onDialog = vi.fn();
    function Picker() {
      const [open, setOpen] = useState(false);
      return (
        <Dialog open onOpenChange={onDialog} title="Gallery">
          <Button onPress={() => setOpen(true)}>Pick</Button>
          <Sheet side="bottom" title="Preview" open={open} onOpenChange={setOpen}><Button>Create</Button></Sheet>
        </Dialog>
      );
    }
    render(<Picker />);
    const pick = screen.getByRole('button', { name: 'Pick' });
    await user.click(pick);
    // Not hidden by the dialog: found without `hidden`, and focus moves into it.
    const sheet = await screen.findByRole('dialog', { name: 'Preview' });
    await waitFor(() => expect(sheet).toContainElement(document.activeElement as HTMLElement));
    const create = within(sheet).getByRole('button', { name: 'Create' });
    await user.click(create);
    expect(create).toHaveFocus();
    // Tab keeps to the sheet: past its last control to the first (Close), and back.
    await user.tab();
    expect(within(sheet).getByRole('button', { name: 'Close' })).toHaveFocus();
    await user.tab({ shift: true });
    expect(create).toHaveFocus();
    await user.keyboard('{Escape}');
    await waitFor(() => expect(screen.queryByRole('dialog', { name: 'Preview' })).not.toBeInTheDocument());
    expect(onDialog).not.toHaveBeenCalled();
    await waitFor(() => expect(pick).toHaveFocus());
  });

  it('keeps a hidden title for screen readers', async () => {
    render(
      <Sheet side="right" title="Sidebar" hideTitle open footer={<button type="button">Settings</button>}>
        body
      </Sheet>,
    );
    const sheet = await screen.findByRole('dialog', { name: 'Sidebar' });
    expect(within(sheet).getByRole('button', { name: 'Settings' })).toBeInTheDocument();
  });

  it('renders nothing while closed', () => {
    render(<Sheet title="Sidebar" open={false}>body</Sheet>);
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });
});
