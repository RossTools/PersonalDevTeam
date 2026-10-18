import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { TicketKey } from './TicketKey';

describe('TicketKey', () => {
  it('renders the key in a mono chip', () => {
    render(<TicketKey ticketKey="PDT-42" />);
    expect(screen.getByText('PDT-42').closest('span[class*="font-mono"]')).not.toBeNull();
    expect(screen.queryByRole('button')).toBeNull();
  });

  it('is a button with onClick', async () => {
    const onClick = vi.fn();
    render(<TicketKey ticketKey="PDT-42" title="Fix login" onClick={onClick} />);
    await userEvent.click(screen.getByRole('button', { name: 'PDT-42' }));
    expect(onClick).toHaveBeenCalled();
  });

  it('is a same-tab link with an app href', () => {
    render(<TicketKey ticketKey="PDT-42" href="/tickets/PDT-42" />);
    const link = screen.getByRole('link', { name: 'PDT-42' });
    expect(link).toHaveAttribute('href', '/tickets/PDT-42');
    expect(link).not.toHaveAttribute('target');
  });
});
