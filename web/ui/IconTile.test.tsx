import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { IconTile } from './IconTile';

describe('IconTile', () => {
  it('shows its icon in its tone, hidden when the row says it already', () => {
    const { container } = render(<><IconTile tone="failed" size="sm" aria-hidden><svg /></IconTile><IconTile><svg aria-label="Sheet" /></IconTile></>);
    const [hidden, shown] = container.querySelectorAll('[data-slot=icon-tile]');
    expect(hidden).toHaveAttribute('aria-hidden', 'true');
    expect(hidden).toHaveClass('bg-failed-soft');
    expect(shown).not.toHaveAttribute('aria-hidden');
    expect(screen.getByLabelText('Sheet')).toBeInTheDocument();
  });
});
