import { act, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { HoverCard } from './HoverCard';

afterEach(() => vi.useRealTimers());

describe('HoverCard', () => {
  it('opens a moment after the pointer arrives and closes when it leaves', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    const onOpenChange = vi.fn();
    render(
      <HoverCard content="Save the cart across visits" onOpenChange={onOpenChange}>
        <a href="/tickets/SHOP-19">SHOP-19</a>
      </HoverCard>,
    );
    await user.click(document.body); // React Aria opens hover cards only when the pointer is the current modality
    await user.hover(screen.getByRole('link', { name: 'SHOP-19' }));
    expect(screen.queryByRole('tooltip')).toBeNull();
    act(() => vi.advanceTimersByTime(500));
    expect(screen.getByRole('tooltip')).toHaveTextContent('Save the cart across visits');
    expect(onOpenChange).toHaveBeenLastCalledWith(true);
    await user.unhover(screen.getByRole('link', { name: 'SHOP-19' }));
    act(() => vi.advanceTimersByTime(500));
    expect(screen.queryByRole('tooltip')).toBeNull();
  });

  it('opens on keyboard focus, describes its trigger and closes on Escape', async () => {
    render(
      <HoverCard content="In progress · medium">
        <a href="/tickets/SHOP-19">SHOP-19</a>
      </HoverCard>,
    );
    await userEvent.tab();
    const card = await screen.findByRole('tooltip');
    expect(screen.getByRole('link', { name: 'SHOP-19' })).toHaveAttribute('aria-describedby', card.id);
    await userEvent.keyboard('{Escape}');
    expect(screen.queryByRole('tooltip')).toBeNull();
  });
});
