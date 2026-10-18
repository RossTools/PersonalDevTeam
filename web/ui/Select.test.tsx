import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useState } from 'react';
import { describe, expect, it, vi } from 'vitest';
import { Select } from './Select';

const options = [
  { value: 'claude', label: 'Claude Code' },
  { value: 'codex', label: 'Codex' },
  { value: 'shell', label: 'Shell', disabled: true },
];

function Host({ onChange = () => {} }: { onChange?: (v: string) => void }) {
  const [v, setV] = useState('claude');
  return (
    <Select
      aria-label="Agent"
      options={options}
      value={v}
      onChange={(next) => {
        setV(next);
        onChange(next);
      }}
    />
  );
}

describe('Select', () => {
  it('shows the selected label on a button named by aria-label', () => {
    render(<Host />);
    const trigger = screen.getByRole('button', { name: /Agent/ });
    expect(trigger).toHaveTextContent('Claude Code');
    expect(trigger.className).toContain('h-control-md');
  });

  it('opens with a click, selects with a click and calls onChange(value)', async () => {
    const onChange = vi.fn();
    render(<Host onChange={onChange} />);
    await userEvent.click(screen.getByRole('button', { name: /Agent/ }));
    const list = screen.getByRole('listbox');
    expect(within(list).getByRole('option', { name: 'Shell' })).toHaveAttribute('aria-disabled', 'true');
    await userEvent.click(within(list).getByRole('option', { name: 'Codex' }));
    expect(onChange).toHaveBeenCalledWith('codex');
    expect(screen.queryByRole('listbox')).toBeNull();
    expect(screen.getByRole('button', { name: /Agent/ })).toHaveTextContent('Codex');
  });

  it('keyboard: ArrowDown opens, arrows move, Enter selects, Escape closes and returns focus', async () => {
    const onChange = vi.fn();
    const user = userEvent.setup();
    render(<Host onChange={onChange} />);
    await user.tab();
    const trigger = screen.getByRole('button', { name: /Agent/ });
    expect(trigger).toHaveFocus();
    await user.keyboard('{ArrowDown}');
    expect(screen.getByRole('listbox')).toBeInTheDocument();
    await user.keyboard('{Escape}');
    expect(screen.queryByRole('listbox')).toBeNull();
    await waitFor(() => expect(trigger).toHaveFocus());
    await user.keyboard('{ArrowDown}');
    await user.keyboard('{ArrowDown}{Enter}');
    expect(onChange).toHaveBeenCalledWith('codex');
    await waitFor(() => expect(trigger).toHaveFocus());
  });

  it('shows the placeholder with no value, and sm size', () => {
    render(<Select aria-label="Branch" size="sm" options={options} value={null} onChange={() => {}} placeholder="Pick one" />);
    const trigger = screen.getByRole('button', { name: /Branch/ });
    expect(trigger).toHaveTextContent('Pick one');
    expect(trigger.className).toContain('h-control-sm');
  });

  it('disabled', () => {
    render(<Select aria-label="X" disabled options={options} value="claude" onChange={() => {}} />);
    expect(screen.getByRole('button', { name: /X/ })).toBeDisabled();
  });
});
