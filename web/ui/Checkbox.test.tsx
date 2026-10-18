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
});
