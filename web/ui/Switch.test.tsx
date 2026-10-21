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

  // React Aria's input is visually hidden with position: absolute. Unless the label positions it, it sits outside the
  // page's scroller, and focusing it on a click scrolls the app shell to reach it (PDT-87: the whole UI shifted).
  it('keeps its hidden input positioned by its own label', () => {
    render(<Switch label="Merge through pull requests" />);
    const label = screen.getByRole('switch', { name: 'Merge through pull requests' }).closest('label')!;
    expect(label.className.split(' ')).toContain('relative');
  });
});
