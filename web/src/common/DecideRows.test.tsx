import { render, screen, within } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import type { DecideRow } from '../api/gen/DecideRow';
import { DecideRows } from './DecideRows';

describe('DecideRows', () => {
  it('repeats the note under the decision for small screens, where the Note column is hidden (PDT-548)', () => {
    const row = { itemId: 'i1', ticketId: 't1', ticketKey: 'PDT-1', ticketTitle: 'Proposal', decision: 'reject', agentic: false, note: 'Not worth it' } as DecideRow;
    render(<DecideRows rows={[row]} kept={() => true} onToggle={() => {}} />);
    const [, , decision, note] = within(screen.getAllByRole('row')[1]).getAllByRole('cell');
    expect(note).toHaveClass('max-sm:hidden');
    expect(within(decision).getByText('Not worth it')).toHaveClass('sm:hidden');
  });
});
