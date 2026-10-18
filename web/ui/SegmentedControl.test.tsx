import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useState } from 'react';
import { describe, expect, it, vi } from 'vitest';
import { SegmentedControl } from './SegmentedControl';

const options = [
  { value: 'folder', label: 'Folder' },
  { value: 'clone', label: 'Clone URL' },
  { value: 'other', label: 'Other', disabled: true },
] as const;

function Host({ onChange }: { onChange: (v: string) => void }) {
  const [v, setV] = useState<string>('folder');
  return (
    <SegmentedControl
      aria-label="Source"
      options={[...options]}
      value={v}
      onChange={(next) => {
        setV(next);
        onChange(next);
      }}
    />
  );
}

describe('SegmentedControl', () => {
  it('is a single-selection group with the value selected', () => {
    render(<Host onChange={() => {}} />);
    const group = screen.getByRole('radiogroup', { name: 'Source' });
    expect(group).toBeInTheDocument();
    expect(screen.getByRole('radio', { name: 'Folder' })).toBeChecked();
    expect(screen.getByRole('radio', { name: 'Clone URL' })).not.toBeChecked();
  });

  it('click selects; clicking the selected one keeps it', async () => {
    const onChange = vi.fn();
    render(<Host onChange={onChange} />);
    await userEvent.click(screen.getByRole('radio', { name: 'Clone URL' }));
    expect(onChange).toHaveBeenLastCalledWith('clone');
    expect(screen.getByRole('radio', { name: 'Clone URL' })).toBeChecked();
    await userEvent.click(screen.getByRole('radio', { name: 'Clone URL' }));
    expect(screen.getByRole('radio', { name: 'Clone URL' })).toBeChecked();
    expect(onChange).toHaveBeenCalledTimes(1);
  });

  it('one tab stop; arrow keys move focus, Space/Enter select', async () => {
    const onChange = vi.fn();
    render(
      <>
        <Host onChange={onChange} />
        <button>after</button>
      </>,
    );
    await userEvent.tab();
    expect(screen.getByRole('radio', { name: 'Folder' })).toHaveFocus();
    await userEvent.keyboard('{ArrowRight}');
    expect(screen.getByRole('radio', { name: 'Clone URL' })).toHaveFocus();
    await userEvent.keyboard(' ');
    expect(onChange).toHaveBeenLastCalledWith('clone');
    await userEvent.keyboard('{ArrowLeft}');
    expect(screen.getByRole('radio', { name: 'Folder' })).toHaveFocus();
    await userEvent.keyboard('{Enter}');
    expect(onChange).toHaveBeenLastCalledWith('folder');
    await userEvent.tab();
    expect(screen.getByRole('button', { name: 'after' })).toHaveFocus();
  });
});
