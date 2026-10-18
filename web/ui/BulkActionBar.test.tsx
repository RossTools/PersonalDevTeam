import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { BulkActionBar } from './BulkActionBar';

describe('BulkActionBar', () => {
  it('shows the count, the actions and a fixed height', () => {
    render(
      <BulkActionBar count={3} onClear={() => {}}>
        <button type="button">Move to Ready</button>
      </BulkActionBar>,
    );
    const bar = screen.getByRole('toolbar', { name: 'Bulk actions' });
    expect(bar).toHaveTextContent('3 selected');
    expect(bar.className).toContain('h-10');
    expect(screen.getByRole('button', { name: 'Move to Ready' })).toBeInTheDocument();
  });

  it('uses the noun', () => {
    render(<BulkActionBar count={2} noun="tickets" onClear={() => {}} />);
    expect(screen.getByText('2 tickets selected')).toBeInTheDocument();
  });

  it('clears with the button and with Escape', async () => {
    const onClear = vi.fn();
    render(
      <BulkActionBar count={1} onClear={onClear}>
        <button type="button">Delete…</button>
      </BulkActionBar>,
    );
    await userEvent.click(screen.getByRole('button', { name: 'Clear' }));
    expect(onClear).toHaveBeenCalledTimes(1);
    screen.getByRole('button', { name: 'Delete…' }).focus();
    await userEvent.keyboard('{Escape}');
    expect(onClear).toHaveBeenCalledTimes(2);
  });
});
