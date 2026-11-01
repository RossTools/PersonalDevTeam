import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { Callout } from './Callout';

describe('Callout', () => {
  it('shows its title over its body, coloured by tone down its left edge', () => {
    render(<Callout tone="error" title="1 check failed" aria-label="CI">test (ubuntu)</Callout>);
    const region = screen.getByRole('region', { name: 'CI' });
    expect(region).toHaveTextContent('1 check failed');
    expect(region).toHaveTextContent('test (ubuntu)');
    expect(region.className).toContain('border-l-failed');
    expect(region).toHaveAttribute('data-tone', 'error');
  });

  it('is info by default and announces nothing', () => {
    render(<Callout>Fine</Callout>);
    expect(screen.queryByRole('alert')).toBeNull();
    expect(screen.getByText('Fine').getAttribute('data-tone')).toBe('info');
  });
});
