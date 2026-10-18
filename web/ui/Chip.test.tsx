import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { GitBranch } from 'lucide-react';
import { Chip } from './Chip';

describe('Chip', () => {
  it('is a mono neutral label', () => {
    render(<Chip icon={<GitBranch aria-hidden />}>main</Chip>);
    const chip = screen.getByText('main').parentElement!;
    expect(chip.tagName).toBe('SPAN');
    expect(chip.className).toContain('font-mono');
    expect(chip.querySelector('svg')).not.toBeNull();
  });

  it('with an external href is a link that opens in a new tab', () => {
    render(<Chip href="https://github.com/o/r/tree/main">main</Chip>);
    const link = screen.getByRole('link', { name: 'main' });
    expect(link).toHaveAttribute('href', 'https://github.com/o/r/tree/main');
    expect(link).toHaveAttribute('target', '_blank');
    expect(link).toHaveAttribute('rel', 'noreferrer');
  });

  it('internal links stay in the tab', () => {
    render(<Chip href="/tickets/PDT-42">PDT-42</Chip>);
    expect(screen.getByRole('link', { name: 'PDT-42' })).not.toHaveAttribute('target');
  });
});
