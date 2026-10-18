import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useState } from 'react';
import { describe, expect, it } from 'vitest';
import { Input } from './Input';

describe('Input', () => {
  it('works as a controlled native input', async () => {
    function Host() {
      const [v, setV] = useState('');
      return (
        <>
          <Input aria-label="Branch" value={v} onChange={(e) => setV(e.target.value)} />
          <output>{v}</output>
        </>
      );
    }
    render(<Host />);
    await userEvent.type(screen.getByRole('textbox', { name: 'Branch' }), 'feat/x');
    expect(screen.getByRole('status')).toHaveTextContent('feat/x');
  });

  it('sizes match Button (md default, sm) and grow on touch; mono and className apply', () => {
    render(
      <>
        <Input aria-label="A" />
        <Input aria-label="B" size="sm" mono className="w-40" />
      </>,
    );
    const a = screen.getByRole('textbox', { name: 'A' });
    expect(a.className).toContain('h-control-md');
    expect(a.className).toContain('pointer-coarse:h-control-touch');
    const b = screen.getByRole('textbox', { name: 'B' });
    expect(b.className).toContain('h-control-sm');
    expect(b.className).toContain('font-mono');
    expect(b.className).toContain('w-40');
    expect(b).not.toHaveAttribute('size');
  });

  it('forwards disabled, placeholder and aria-invalid', () => {
    render(<Input aria-label="C" disabled placeholder="name" aria-invalid />);
    const c = screen.getByRole('textbox', { name: 'C' });
    expect(c).toBeDisabled();
    expect(c).toHaveAttribute('placeholder', 'name');
    expect(c).toHaveAttribute('aria-invalid', 'true');
  });
});
