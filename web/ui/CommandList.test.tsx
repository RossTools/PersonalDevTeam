import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useState } from 'react';
import { describe, expect, it, vi } from 'vitest';
import { CommandList, type CommandItem } from './CommandList';

const items: CommandItem[] = [
  { id: 'new', label: 'New tab', shortcut: 'Ctrl+] c' },
  { id: 'pdt-main', label: 'main', group: 'PersonalDevTeam', keywords: ['pdt'] },
  { id: 'pdt-ui', label: 'phase-3c-1-ui', group: 'PersonalDevTeam', keywords: ['pdt'] },
  { id: 'demo-main', label: 'trunk', group: 'Demo' },
];

describe('CommandList', () => {
  it('lists items in groups with shortcut keys, and focuses the filter', () => {
    render(<CommandList label="Places" items={items} onAction={() => {}} />);
    const list = screen.getByRole('menu', { name: 'Places' });
    expect(within(list).getByRole('menuitem', { name: /New tab/ })).toBeInTheDocument();
    expect(within(list).getByText('PersonalDevTeam')).toBeInTheDocument();
    expect(within(list).getByText('Demo')).toBeInTheDocument();
    expect(within(list).getByText('Ctrl+] c').tagName).toBe('KBD');
    expect(screen.getByRole('searchbox')).toHaveFocus();
  });

  it('filters by label and keywords, with the empty state inside the list', async () => {
    const user = userEvent.setup();
    render(<CommandList label="Places" items={items} onAction={() => {}} />);
    await user.keyboard('pdt');
    const list = screen.getByRole('menu', { name: 'Places' });
    expect(within(list).getAllByRole('menuitem').map((e) => e.textContent)).toEqual(['main', 'phase-3c-1-ui']);
    expect(within(list).queryByText('Demo')).not.toBeInTheDocument();
    await user.keyboard('zzz');
    expect(within(screen.getByRole('menu', { name: 'Places' })).getByText('No results')).toBeInTheDocument();
  });

  it('moves with the arrow keys and chooses with Enter', async () => {
    const user = userEvent.setup();
    const onAction = vi.fn();
    render(<CommandList label="Places" items={items} onAction={onAction} />);
    await user.keyboard('{ArrowDown}{ArrowDown}{Enter}');
    expect(onAction).toHaveBeenCalledWith('pdt-main');
  });

  it('chooses with a click', async () => {
    const user = userEvent.setup();
    const onAction = vi.fn();
    render(<CommandList label="Places" items={items} onAction={onAction} />);
    await user.click(screen.getByRole('menuitem', { name: 'trunk' }));
    expect(onAction).toHaveBeenCalledWith('demo-main');
  });

  it('takes a controlled query', async () => {
    const user = userEvent.setup();
    function Controlled() {
      const [q, setQ] = useState('trunk');
      return (
        <>
          <CommandList label="Places" items={items} onAction={() => {}} query={q} onQueryChange={setQ} />
          <output aria-label="query">{q}</output>
        </>
      );
    }
    render(<Controlled />);
    expect(screen.getAllByRole('menuitem').map((e) => e.textContent)).toEqual(['trunk']);
    await user.clear(screen.getByRole('searchbox'));
    await user.keyboard('main');
    expect(screen.getByRole('status', { name: 'query' })).toHaveTextContent('main');
    expect(screen.getAllByRole('menuitem').map((e) => e.textContent)).toEqual(['main']);
  });
});
