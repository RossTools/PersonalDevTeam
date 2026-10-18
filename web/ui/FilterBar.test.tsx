import { useState } from 'react';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { FilterBar } from './FilterBar';

function Bar({ slash = true }: { slash?: boolean }) {
  const [q, setQ] = useState('');
  return (
    <>
      <input aria-label="Other field" />
      <button type="button">Elsewhere</button>
      <FilterBar search={q} onSearchChange={setQ} searchLabel="Search tickets" slashToFocus={slash}
        trailing={<button type="button">Sort</button>}>
        <button type="button">Stage</button>
      </FilterBar>
    </>
  );
}

describe('FilterBar', () => {
  it('renders search, chips and the trailing slot in a search landmark', () => {
    render(<Bar />);
    const bar = screen.getByRole('search');
    expect(bar).toContainElement(screen.getByRole('searchbox', { name: 'Search tickets' }));
    expect(bar).toContainElement(screen.getByRole('button', { name: 'Stage' }));
    expect(bar).toContainElement(screen.getByRole('button', { name: 'Sort' }));
    expect(screen.getByText('/')).toBeInTheDocument();
  });

  it('reports typing and clears with the clear button and Escape', async () => {
    const onSearchChange = vi.fn();
    const { rerender } = render(<FilterBar search="" onSearchChange={onSearchChange} />);
    await userEvent.type(screen.getByRole('searchbox'), 'a');
    expect(onSearchChange).toHaveBeenLastCalledWith('a');
    rerender(<FilterBar search="abc" onSearchChange={onSearchChange} />);
    await userEvent.click(screen.getByRole('button', { name: 'Clear search' }));
    expect(onSearchChange).toHaveBeenLastCalledWith('');
    screen.getByRole('searchbox').focus();
    await userEvent.keyboard('{Escape}');
    expect(onSearchChange).toHaveBeenLastCalledWith('');
  });

  it('focuses search on / unless typing in another field', async () => {
    render(<Bar />);
    screen.getByRole('button', { name: 'Elsewhere' }).focus();
    await userEvent.keyboard('/');
    const search = screen.getByRole('searchbox', { name: 'Search tickets' });
    expect(search).toHaveFocus();
    expect(search).toHaveValue('');
    await userEvent.keyboard('{Escape}');
    expect(search).not.toHaveFocus();

    await userEvent.click(screen.getByRole('textbox', { name: 'Other field' }));
    await userEvent.keyboard('/');
    expect(screen.getByRole('textbox', { name: 'Other field' })).toHaveValue('/');
    expect(search).not.toHaveFocus();
  });

  it('ignores / when slashToFocus is off', async () => {
    render(<Bar slash={false} />);
    screen.getByRole('button', { name: 'Elsewhere' }).focus();
    await userEvent.keyboard('/');
    expect(screen.getByRole('searchbox')).not.toHaveFocus();
  });
});
