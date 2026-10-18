import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useRef } from 'react';
import { describe, expect, it, vi } from 'vitest';
import { Popover } from './Popover';
import { Button } from './primitives/button';

describe('Popover', () => {
  it('opens from its trigger, closes on Escape and returns focus', async () => {
    const user = userEvent.setup();
    render(
      <Popover trigger={<Button>Filters</Button>} label="Filters">
        <input aria-label="Name" />
      </Popover>,
    );
    const trigger = screen.getByRole('button', { name: 'Filters' });
    expect(trigger).toHaveAttribute('aria-expanded', 'false');
    await user.click(trigger);
    const panel = screen.getByRole('dialog', { name: 'Filters' });
    expect(within(panel).getByRole('textbox', { name: 'Name' })).toBeInTheDocument();
    await user.keyboard('{Escape}');
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    await waitFor(() => expect(trigger).toHaveFocus());
  });

  it('is named by a visible title and gives children a close function', async () => {
    const user = userEvent.setup();
    const picked = vi.fn();
    render(
      <Popover trigger={<button type="button">Place</button>} title="Choose a place">
        {({ close }) => (
          <button type="button" onClick={() => { picked(); close(); }}>
            main
          </button>
        )}
      </Popover>,
    );
    await user.click(screen.getByRole('button', { name: 'Place' }));
    expect(screen.getByRole('dialog', { name: 'Choose a place' })).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'main' }));
    expect(picked).toHaveBeenCalledOnce();
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });

  it('can be controlled', async () => {
    const user = userEvent.setup();
    const onOpenChange = vi.fn();
    const { rerender } = render(
      <Popover trigger={<Button>Filters</Button>} label="Filters" open={false} onOpenChange={onOpenChange}>
        body
      </Popover>,
    );
    await user.click(screen.getByRole('button', { name: 'Filters' }));
    expect(onOpenChange).toHaveBeenCalledWith(true);
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    rerender(
      <Popover trigger={<Button>Filters</Button>} label="Filters" open onOpenChange={onOpenChange}>
        body
      </Popover>,
    );
    expect(screen.getByRole('dialog', { name: 'Filters' })).toBeInTheDocument();
  });

  it('anchors to an element elsewhere and closes on Escape', async () => {
    const user = userEvent.setup();
    const onOpenChange = vi.fn();
    function Anchored() {
      const ref = useRef<HTMLButtonElement>(null);
      return (
        <>
          <button ref={ref} type="button">Plus</button>
          <Popover anchor={ref} label="Chooser" open onOpenChange={onOpenChange}>
            <input aria-label="Filter" />
          </Popover>
        </>
      );
    }
    render(<Anchored />);
    const panel = screen.getByRole('dialog', { name: 'Chooser' });
    expect(within(panel).getByRole('textbox', { name: 'Filter' })).toBeInTheDocument();
    await user.click(within(panel).getByRole('textbox', { name: 'Filter' }));
    await user.keyboard('{Escape}');
    expect(onOpenChange).toHaveBeenCalledWith(false);
  });
});
