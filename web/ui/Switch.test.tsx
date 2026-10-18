import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { Switch } from './Switch';

describe('Switch', () => {
  it('is a switch named by its label; click and Space call onChange(checked)', async () => {
    const onChange = vi.fn();
    render(<Switch label="Skip permissions" checked={false} onChange={onChange} />);
    const sw = screen.getByRole('switch', { name: 'Skip permissions' });
    expect(sw).not.toBeChecked();
    await userEvent.click(screen.getByText('Skip permissions'));
    expect(onChange).toHaveBeenLastCalledWith(true);
    sw.focus();
    await userEvent.keyboard(' ');
    expect(onChange).toHaveBeenCalledTimes(2);
  });

  it('controlled checked, disabled, children as label', () => {
    render(
      <Switch checked disabled>
        Remote access
      </Switch>,
    );
    const sw = screen.getByRole('switch', { name: 'Remote access' });
    expect(sw).toBeChecked();
    expect(sw).toBeDisabled();
  });
});
