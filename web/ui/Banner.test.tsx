import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { Banner } from './Banner';

describe('Banner', () => {
  it('is a fixed-height strip that never wraps', () => {
    render(<Banner tone="accent" actions={<button>Update</button>}>Update to v0.1.99 available</Banner>);
    const strip = screen.getByText('Update to v0.1.99 available').parentElement!;
    for (const c of ['h-8', 'shrink-0', 'whitespace-nowrap', 'overflow-hidden', 'flex-nowrap', 'bg-accent']) {
      expect(strip.className).toContain(c);
    }
    expect(screen.getByText('Update to v0.1.99 available').className).toContain('truncate');
    expect(screen.getByRole('button', { name: 'Update' })).toBeInTheDocument();
  });

  it('failed is an alert, running/waiting a status, neutral has no role', () => {
    const { rerender } = render(<Banner tone="failed">Update failed</Banner>);
    expect(screen.getByRole('alert')).toHaveTextContent('Update failed');
    rerender(<Banner tone="waiting">Installing</Banner>);
    expect(screen.getByRole('status')).toHaveTextContent('Installing');
    rerender(<Banner>Plain</Banner>);
    expect(screen.queryByRole('status')).toBeNull();
    expect(screen.queryByRole('alert')).toBeNull();
  });
});
