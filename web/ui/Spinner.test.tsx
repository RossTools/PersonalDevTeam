import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { Spinner } from './Spinner';

describe('Spinner', () => {
  it('is a status named Loading by default, or by label', () => {
    render(
      <>
        <Spinner />
        <Spinner label="Cloning" />
      </>,
    );
    expect(screen.getByRole('status', { name: 'Loading' })).toBeInTheDocument();
    const s = screen.getByRole('status', { name: 'Cloning' });
    expect(s.getAttribute('class')).toContain('motion-reduce:animate-none');
  });

  it('decorative spinners are hidden from assistive tech', () => {
    const { container } = render(<Spinner decorative />);
    expect(screen.queryByRole('status')).toBeNull();
    expect(container.querySelector('svg')).toHaveAttribute('aria-hidden', 'true');
  });
});
