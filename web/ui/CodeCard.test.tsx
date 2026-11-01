import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { CodeCard } from './CodeCard';

describe('CodeCard', () => {
  it('shows what runs and why over the code, kept as written', () => {
    render(<CodeCard aria-label="Command" title="Bash · in pdt-SHOP-23" aside="Reset the dev database" code={'npm run db:reset\n  && seed'} />);
    const card = screen.getByRole('group', { name: 'Command' });
    expect(card).toHaveTextContent('Bash · in pdt-SHOP-23');
    expect(card).toHaveTextContent('Reset the dev database');
    expect(card.querySelector('pre')?.textContent).toBe('npm run db:reset\n  && seed');
  });

  it('takes other content in place of the code', () => {
    render(<CodeCard aria-label="Edit"><div>a diff</div></CodeCard>);
    expect(screen.getByRole('group', { name: 'Edit' }).querySelector('pre')).toBeNull();
    expect(screen.getByText('a diff')).toBeInTheDocument();
  });
});
