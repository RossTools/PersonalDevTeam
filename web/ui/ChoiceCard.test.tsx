import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useState } from 'react';
import { describe, expect, it, vi } from 'vitest';
import { ChoiceCard, ChoiceCardGroup } from './ChoiceCard';

describe('ChoiceCard', () => {
  it('is a button that says whether it is picked', async () => {
    const onClick = vi.fn();
    const { rerender } = render(<ChoiceCard title="09:00" description="Morning" onClick={onClick} />);
    const card = screen.getByRole('button', { name: /09:00/ });
    expect(card).toHaveAttribute('aria-pressed', 'false');
    await userEvent.click(card);
    expect(onClick).toHaveBeenCalledOnce();
    rerender(<ChoiceCard title="09:00" description="Morning" selected onClick={onClick} />);
    expect(card).toHaveAttribute('aria-pressed', 'true');
    expect(card.className).toContain('border-primary');
  });

  it('current: aria-current instead of aria-pressed', () => {
    render(<ChoiceCard selection="current" selected aria-label="Session">Body</ChoiceCard>);
    const card = screen.getByRole('button', { name: 'Session' });
    expect(card).toHaveAttribute('aria-current', 'true');
    expect(card).not.toHaveAttribute('aria-pressed');
  });

  it('a disabled card ignores presses', async () => {
    const onClick = vi.fn();
    render(<ChoiceCard title="Codex" disabled onClick={onClick} />);
    await userEvent.click(screen.getByRole('button', { name: 'Codex' }));
    expect(onClick).not.toHaveBeenCalled();
  });
});

describe('ChoiceCardGroup', () => {
  function Demo({ onChange }: { onChange: (v: string) => void }) {
    const [v, setV] = useState('claude');
    return (
      <ChoiceCardGroup
        aria-label="Default agent"
        value={v}
        onChange={(n) => { setV(n); onChange(n); }}
        options={[
          { value: 'claude', title: 'Claude Code', description: 'Found' },
          { value: 'codex', title: 'Codex', description: 'Found' },
          { value: 'devin', title: 'Devin', description: 'Not installed', disabled: true },
        ]}
      />
    );
  }

  it('is a radio group: a click picks, arrows move and pick, disabled ones are skipped', async () => {
    const onChange = vi.fn();
    render(<Demo onChange={onChange} />);
    expect(screen.getByRole('radiogroup', { name: 'Default agent' })).toBeInTheDocument();
    expect(screen.getByRole('radio', { name: /Claude Code/ })).toBeChecked();
    await userEvent.click(screen.getByText('Codex'));
    expect(onChange).toHaveBeenLastCalledWith('codex');
    expect(screen.getByRole('radio', { name: /Codex/ })).toHaveFocus();
    await userEvent.keyboard('{ArrowRight}');
    expect(onChange).toHaveBeenLastCalledWith('claude');
    expect(screen.getByRole('radio', { name: /Devin/ })).toBeDisabled();
  });
});
