import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { PriorityIcon } from './PriorityIcon';

describe('PriorityIcon', () => {
  it('names the priority for screen readers only by default', () => {
    render(<PriorityIcon priority="high" />);
    expect(screen.getByText('High').className).toContain('sr-only');
  });

  it('shows the label when asked', () => {
    render(<PriorityIcon priority="low" showLabel />);
    expect(screen.getByText('Low').className).not.toContain('sr-only');
  });

  it('gives only urgent a state colour', () => {
    const { container, rerender } = render(<PriorityIcon priority="urgent" />);
    expect(container.firstElementChild!.className).toContain('text-waiting');
    rerender(<PriorityIcon priority="medium" />);
    expect(container.firstElementChild!.className).not.toMatch(/text-(waiting|failed|running)/);
    rerender(<PriorityIcon priority="none" />);
    expect(screen.getByText('No priority')).toBeInTheDocument();
  });
});
