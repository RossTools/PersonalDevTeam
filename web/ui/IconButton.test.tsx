import { fireEvent, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { IconButton } from './IconButton';
import { Kbd } from './Kbd';

const Icon = () => <svg data-testid="icon" />;

describe('IconButton', () => {
  it('is named by its label and square-sized (sm 24, md 28, touch 44)', () => {
    render(
      <>
        <IconButton label="Close tab">
          <Icon />
        </IconButton>
        <IconButton label="Small" size="sm">
          <Icon />
        </IconButton>
      </>,
    );
    const b = screen.getByRole('button', { name: 'Close tab' });
    expect(b.className).toContain('size-control-md');
    expect(b.className).toContain('pointer-coarse:size-control-touch');
    expect(b.className).toContain('text-muted-foreground'); // ghost by default
    expect(screen.getByRole('button', { name: 'Small' }).className).toContain('size-control-sm');
  });

  it('shows its label as a tooltip on keyboard focus (with an optional shortcut), Escape hides it', async () => {
    render(
      <IconButton label="New tab" shortcut={<Kbd>T</Kbd>}>
        <Icon />
      </IconButton>,
    );
    await userEvent.tab();
    expect(screen.getByRole('button', { name: 'New tab' })).toHaveFocus();
    const tip = await screen.findByRole('tooltip');
    expect(tip).toHaveTextContent('New tab');
    expect(tip.querySelector('kbd')).toHaveTextContent('T');
    await userEvent.keyboard('{Escape}');
    expect(screen.queryByRole('tooltip')).toBeNull();
    expect(screen.getByRole('button', { name: 'New tab' })).toHaveFocus();
  });

  it('pressed sets aria-pressed for toggles', () => {
    const { rerender } = render(
      <IconButton label="Maximize" pressed={false}>
        <Icon />
      </IconButton>,
    );
    expect(screen.getByRole('button', { name: 'Maximize' })).toHaveAttribute('aria-pressed', 'false');
    rerender(
      <IconButton label="Maximize" pressed>
        <Icon />
      </IconButton>,
    );
    expect(screen.getByRole('button', { name: 'Maximize' })).toHaveAttribute('aria-pressed', 'true');
  });

  it('onClick works with click and keyboard; disabled blocks it', async () => {
    const onClick = vi.fn();
    const { rerender } = render(
      <IconButton label="Stop" onClick={onClick}>
        <Icon />
      </IconButton>,
    );
    fireEvent.click(screen.getByRole('button', { name: 'Stop' }));
    await userEvent.click(screen.getByRole('button', { name: 'Stop' }));
    expect(onClick).toHaveBeenCalledTimes(2);
    rerender(
      <IconButton label="Stop" onClick={onClick} disabled>
        <Icon />
      </IconButton>,
    );
    expect(screen.getByRole('button', { name: 'Stop' })).toBeDisabled();
  });
});
