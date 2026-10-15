import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { Button, StatusDot } from '@ui';

describe('Button', () => {
  it('primary uses accent tokens', () => {
    render(<Button variant="primary">Send feedback</Button>);
    const b = screen.getByRole('button', { name: 'Send feedback' });
    expect(b.className).toContain('bg-accent');
    expect(b.className).toContain('text-on-accent');
  });

  it('defaults to secondary and forwards props', () => {
    render(<Button disabled type="submit">Approve</Button>);
    const b = screen.getByRole('button', { name: 'Approve' });
    expect(b).toBeDisabled();
    expect(b).toHaveAttribute('type', 'submit');
    expect(b.className).toContain('border-border');
  });
});

describe('StatusDot', () => {
  it('labels and colors by status', () => {
    render(<StatusDot status="waiting" />);
    const dot = screen.getByRole('img', { name: 'waiting' });
    expect(dot.className).toContain('bg-waiting');
  });
});
