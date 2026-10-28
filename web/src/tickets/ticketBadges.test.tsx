import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { countdown, TicketBadges } from './TicketBadges';
import { summary } from './fixtures';

describe('auto-start countdown', () => {
  it('counts down in minutes and seconds, never below zero', () => {
    expect(countdown(90_000, 0)).toBe('1:30');
    expect(countdown(120_000, 500)).toBe('2:00');
    expect(countdown(5_000, 0)).toBe('0:05');
    expect(countdown(0, 10_000)).toBe('0:00');
  });

  it('shows while a ticket waits out its start delay, and not otherwise', () => {
    const { rerender } = render(<TicketBadges ticket={summary(1, { stage: 'todo', autoStartAt: Date.now() + 90_000 })} />);
    expect(screen.getByText(/Auto-starts in 1:(29|30)/)).toBeInTheDocument();
    rerender(<TicketBadges ticket={summary(1, { stage: 'todo' })} />);
    expect(screen.queryByText(/Auto-starts in/)).not.toBeInTheDocument();
  });
});

describe('the source badge', () => {
  it('says who made a ticket unless the user did, and holds an external one back until let in', () => {
    const { rerender } = render(<TicketBadges ticket={summary(1, {})} />);
    expect(screen.queryByText('External')).not.toBeInTheDocument();
    rerender(<TicketBadges ticket={summary(1, { source: 'pm' })} />);
    expect(screen.getByText('PM')).toBeInTheDocument();
    rerender(<TicketBadges ticket={summary(1, { source: 'external', admitted: false })} />);
    expect(screen.getByText('External')).toHaveAttribute('title', expect.stringContaining('only you can move it to To do'));
    rerender(<TicketBadges ticket={summary(1, { source: 'external', admitted: true })} />);
    expect(screen.getByText('External')).toHaveAttribute('title', expect.stringContaining('you let it in'));
  });
});
