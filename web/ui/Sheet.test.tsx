import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
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
    // A generous timeout: under a full parallel test run the lazy chunk can take over a second to load.
    await waitFor(() => expect(screen.getByRole('button', { name: 'Alpha' })).toHaveAttribute('aria-expanded', 'false'), { timeout: 5000 });
    const trigger = screen.getByRole('button', { name: 'Alpha' });
    await user.click(trigger);
    const sheet = await screen.findByRole('dialog', { name: 'Workspaces' }, { timeout: 5000 });
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
    const sheet = await screen.findByRole('dialog', { name: 'Peek' }, { timeout: 5000 });
    await user.click(within(sheet).getByRole('button', { name: 'Close' }));
    expect(onOpenChange).toHaveBeenCalledWith(false);
  });

  it('closes when modal on a press on its backdrop only, not on one that lands on the page around it', async () => {
    const user = userEvent.setup();
    const onOpenChange = vi.fn();
    render(<Sheet side="left" title="Sidebar" open onOpenChange={onOpenChange}>body</Sheet>);
    await screen.findByRole('dialog', { name: 'Sidebar' }, { timeout: 5000 });
    // Where a click lands when a menu opened on mouse down covers its button by mouse up.
    await user.click(document.body);
    expect(onOpenChange).not.toHaveBeenCalled();
    await user.click(document.querySelector('[data-slot="drawer-viewport"]')!);
    expect(onOpenChange).toHaveBeenCalledWith(false);
  });

  it('stays open when not modal while the page beside it is used', async () => {
    const user = userEvent.setup();
    const onOpenChange = vi.fn();
    render(
      <>
        <button type="button">Beside</button>
        <Sheet side="right" title="Peek" modal={false} open onOpenChange={onOpenChange}>body</Sheet>
      </>,
    );
    const sheet = await screen.findByRole('dialog', { name: 'Peek' }, { timeout: 5000 });
    // The sheet takes focus once it opens; moving focus out of it must not close it either.
    await waitFor(() => expect(sheet.contains(document.activeElement)).toBe(true));
    await user.click(screen.getByRole('button', { name: 'Beside' }));
    expect(screen.getByRole('button', { name: 'Beside' })).toHaveFocus();
    expect(onOpenChange).not.toHaveBeenCalled();
  });

  it('keeps a hidden title for screen readers', async () => {
    render(
      <Sheet side="right" title="Sidebar" hideTitle open footer={<button type="button">Settings</button>}>
        body
      </Sheet>,
    );
    const sheet = await screen.findByRole('dialog', { name: 'Sidebar' }, { timeout: 5000 });
    expect(within(sheet).getByRole('button', { name: 'Settings' })).toBeInTheDocument();
  });

  it('renders nothing while closed', () => {
    render(<Sheet title="Sidebar" open={false}>body</Sheet>);
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });
});
