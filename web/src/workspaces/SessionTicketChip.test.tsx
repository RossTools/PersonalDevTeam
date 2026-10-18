import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { OpenTicketContext } from '../tickets/actions';
import { SessionTicketChip } from './SessionTicketChip';

describe('SessionTicketChip', () => {
  it('opens the ticket without the row or tab around it acting', async () => {
    const open = vi.fn();
    const row = vi.fn();
    render(
      <OpenTicketContext.Provider value={open}>
        <div onClick={row} onPointerDown={row}>
          <SessionTicketChip session={{ projectId: 'p1', ticket: { id: 't1', key: 'ALP-1' } }} />
        </div>
      </OpenTicketContext.Provider>,
    );
    await userEvent.click(screen.getByRole('button', { name: 'ALP-1' }));
    expect(open).toHaveBeenCalledWith({ id: 't1', projectId: 'p1' });
    expect(row).not.toHaveBeenCalled();
  });

  it('renders nothing for a session without a ticket', () => {
    const { container } = render(<SessionTicketChip session={{ projectId: 'p1', ticket: null }} />);
    expect(container).toBeEmptyDOMElement();
  });
});
