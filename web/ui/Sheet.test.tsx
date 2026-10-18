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
