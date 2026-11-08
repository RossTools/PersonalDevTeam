import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { CheckTable } from './CheckTable';

const rows = [{ id: 'a', name: 'One' }, { id: 'b', name: 'Two' }];

describe('CheckTable', () => {
  it('ticks rows, strikes the unticked and reports a change', async () => {
    const onCheck = vi.fn();
    render(
      <CheckTable aria-label="Picks" rows={rows} rowKey={(r) => r.id} columns={[{ id: 'name', header: 'Name', cell: (r) => r.name }, { id: 'n', header: 'Note', wide: true, cell: () => 'x' }]}
        checked={(n) => n === 0} onCheck={onCheck} checkLabel={(r) => `Keep ${r.name}`} />,
    );
    const table = screen.getByRole('table', { name: 'Picks' });
    expect(within(table).getByRole('columnheader', { name: 'Note' })).toHaveClass('max-sm:hidden');
    expect(screen.getByRole('checkbox', { name: 'Keep One' })).toBeChecked();
    expect(screen.getByRole('checkbox', { name: 'Keep Two' })).not.toBeChecked();
    expect(screen.getByText('Two').closest('tr')).toHaveClass('line-through');
    await userEvent.click(screen.getByRole('checkbox', { name: 'Keep One' }));
    expect(onCheck).toHaveBeenCalledWith(0, false);
  });
});
