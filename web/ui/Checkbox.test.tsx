import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useState } from 'react';
import { describe, expect, it, vi } from 'vitest';
import { Checkbox } from './Checkbox';

describe('Checkbox', () => {
  it('toggles on click of the label and on Space, calling onChange(checked)', async () => {
    const onChange = vi.fn();
    function Host() {
      const [c, setC] = useState(false);
      return (
        <Checkbox
          checked={c}
          onChange={(v) => {
            setC(v);
            onChange(v);
          }}
        >
          Also delete the branch
        </Checkbox>
      );
    }
    render(<Host />);
    const box = screen.getByRole('checkbox', { name: 'Also delete the branch' });
    expect(box).not.toBeChecked();
    await userEvent.click(screen.getByText('Also delete the branch'));
    expect(box).toBeChecked();
    expect(onChange).toHaveBeenLastCalledWith(true);
    await userEvent.keyboard(' ');
    expect(box).not.toBeChecked();
    expect(onChange).toHaveBeenLastCalledWith(false);
  });

  it('accepts label as a prop, aria-label without one, disabled and indeterminate', () => {
    render(
      <>
        <Checkbox label="Remote" disabled />
        <Checkbox aria-label="Select all" indeterminate />
      </>,
    );
    expect(screen.getByRole('checkbox', { name: 'Remote' })).toBeDisabled();
    expect(screen.getByRole('checkbox', { name: 'Select all' })).toBePartiallyChecked();
  });

  // React Aria's input is visually hidden with position: absolute. Unless the label positions it, it sits outside the
  // page's scroller, and focusing it on a click scrolls the app shell to reach it (PDT-87: the whole UI shifted).
  it('keeps its hidden input positioned by its own label', () => {
    render(<Checkbox label="Also delete the branch" />);
    const label = screen.getByRole('checkbox', { name: 'Also delete the branch' }).closest('label')!;
    expect(label.className.split(' ')).toContain('relative');
  });

  it('has a round shape that fills when ticked, with a danger border', async () => {
    render(
      <>
        <Checkbox aria-label="Done: Renew the passport" shape="round" danger />
        <Checkbox aria-label="Done: Call the dentist" shape="round" defaultChecked />
      </>,
    );
    const overdue = screen.getByRole('checkbox', { name: 'Done: Renew the passport' }).closest('label')!;
    const dot = overdue.querySelector('[data-slot="checkbox-indicator"]')!;
    expect(dot).toHaveAttribute('data-shape', 'round');
    expect(dot.className).toContain('rounded-full');
    expect(dot.className).toContain('border-destructive');
    const ticked = screen.getByRole('checkbox', { name: 'Done: Call the dentist' });
    expect(ticked).toBeChecked();
    expect(ticked.closest('label')!.querySelector('svg')).not.toBeNull();
    await userEvent.click(ticked);
    expect(ticked).not.toBeChecked();
  });
});
