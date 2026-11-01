import { render, screen, within } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { Facts } from './Facts';

describe('Facts', () => {
  it('shows each figure with what it counts', () => {
    render(<Facts aria-label="Disk space" facts={[{ value: '9 GB', label: 'free' }, { value: 12, label: 'finished worktrees' }]} />);
    const list = screen.getByLabelText('Disk space');
    expect(within(list).getAllByRole('definition').map((d) => d.textContent)).toEqual(['9 GB', '12']);
    expect(within(list).getAllByRole('term').map((d) => d.textContent)).toEqual(['free', 'finished worktrees']);
  });
});
