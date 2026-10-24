import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useState } from 'react';
import { describe, expect, it, vi } from 'vitest';
import { Calendar } from './Calendar';
import { Popover } from './Popover';
import { Button } from './primitives/button';

const cell = (name: RegExp) => screen.getByRole('button', { name });

describe('Calendar', () => {
  it('shows a month, Monday first, with the chosen day selected', () => {
    render(<Calendar value="2026-10-28" onChange={() => {}} />);
    const grid = screen.getByRole('grid');
    const heads = [...grid.querySelectorAll('th')].map((h) => h.textContent);
    expect(heads).toEqual(['M', 'T', 'W', 'T', 'F', 'S', 'S']);
    expect(screen.getByRole('heading').textContent).toMatch(/October 2026/);
    expect(cell(/October 28, 2026/).closest('[data-selected]')).not.toBeNull();
  });

  it('pages with ‹ › and picks a day on click', async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    render(<Calendar value="2026-10-28" onChange={onChange} />);
    await user.click(screen.getByRole('button', { name: 'Next month' }));
    expect(screen.getByRole('heading').textContent).toMatch(/November 2026/);
    await user.click(cell(/November 3, 2026/));
    expect(onChange).toHaveBeenCalledWith('2026-11-03');
  });

  it("doesn't pick days before min, except the chosen one", async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    render(<Calendar value="2026-10-20" min="2026-10-24" onChange={onChange} />);
    expect(cell(/October 22, 2026/).closest('[data-unavailable]')).not.toBeNull();
    expect(cell(/October 20, 2026/).closest('[data-unavailable]')).toBeNull();
    await user.click(cell(/October 22, 2026/));
    expect(onChange).not.toHaveBeenCalled();
    await user.click(cell(/October 25, 2026/));
    expect(onChange).toHaveBeenCalledWith('2026-10-25');
  });

  it('moves with the arrows, picks with Enter, and Escape closes its popover', async () => {
    const user = userEvent.setup();
    const picked: string[] = [];
    function Host() {
      const [v, setV] = useState('2026-10-28');
      return (
        <Popover trigger={<Button>Due</Button>} label="Due date">
          <Calendar value={v} autoFocus onChange={(d) => { picked.push(d); setV(d); }} />
        </Popover>
      );
    }
    render(<Host />);
    const trigger = screen.getByRole('button', { name: 'Due' });
    await user.click(trigger);
    await waitFor(() => expect(cell(/October 28, 2026/)).toHaveFocus());
    await user.keyboard('{ArrowRight}{ArrowDown}');
    expect(cell(/November 5, 2026/)).toHaveFocus();
    await user.keyboard('{Enter}');
    expect(picked).toEqual(['2026-11-05']);
    await user.keyboard('{Escape}');
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    await waitFor(() => expect(trigger).toHaveFocus());
  });
});
