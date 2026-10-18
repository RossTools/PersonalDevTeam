import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { Skeleton } from './Skeleton';

describe('Skeleton', () => {
  it('lines: one bar per line, the last one shorter, one status for all', () => {
    render(<Skeleton lines={3} label="Loading files" />);
    const status = screen.getByRole('status', { name: 'Loading files' });
    const bars = status.querySelectorAll('[data-slot=skeleton]');
    expect(bars).toHaveLength(3);
    expect(bars[2].className).toContain('w-3/5');
    expect(bars[0].className).toContain('w-full');
    for (const b of bars) {
      expect(b).toHaveAttribute('aria-hidden', 'true');
      expect(b.className).toContain('motion-reduce:animate-none');
    }
  });

  it('block: one placeholder sized by className', () => {
    render(<Skeleton className="h-40" />);
    const status = screen.getByRole('status', { name: 'Loading' });
    expect(status.className).toContain('h-40');
    expect(status.className).not.toContain('h-16');
    expect(status.querySelectorAll('[data-slot=skeleton]')).toHaveLength(1);
  });
});
