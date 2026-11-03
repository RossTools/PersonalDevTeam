import { render, screen, within } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { Button } from './Button';
import { SourceCard } from './SourceCard';

describe('SourceCard', () => {
  it('shows the source, its account and its action, with help under them', () => {
    render(
      <SourceCard aria-label="Provider" name="Google Drive" detail="me@example.com" action={<Button>Browse Drive…</Button>}>
        <p>Sign in to browse.</p>
      </SourceCard>,
    );
    const card = screen.getByRole('group', { name: 'Provider' });
    expect(within(card).getByText('Google Drive')).toBeInTheDocument();
    expect(within(card).getByText('me@example.com')).toBeInTheDocument();
    expect(within(card).getByRole('button', { name: 'Browse Drive…' })).toBeInTheDocument();
    expect(within(card).getByText('Sign in to browse.')).toBeInTheDocument();
  });

  it('is no landmark without a label', () => {
    render(<SourceCard name="Local" />);
    expect(screen.queryByRole('group')).toBeNull();
    expect(screen.getByText('Local')).toBeInTheDocument();
  });
});
