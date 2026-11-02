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
    expect(screen.getByText('External').closest('[data-slot=badge]')).toHaveAttribute('title', expect.stringContaining('only you can move it to To do'));
    rerender(<TicketBadges ticket={summary(1, { source: 'external', admitted: true })} />);
    expect(screen.getByText('External').closest('[data-slot=badge]')).toHaveAttribute('title', expect.stringContaining('you let it in'));
  });

  it("names an agent's ticket by its persona and the ticket it was working on", () => {
    const createdBy = { sessionId: 's9', projectId: 'p1', persona: 'Coder', fromTicket: 'PDT-345', fromTicketId: 't345' };
    const { rerender } = render(<TicketBadges ticket={summary(1, { source: 'agent', createdBy })} />);
    expect(screen.getByText('Coder').closest('[data-slot=badge]')).toHaveAttribute('title', 'Created by Coder · from PDT-345');
    rerender(<TicketBadges ticket={summary(1, { source: 'agent' })} />);
    expect(screen.getByText('Agent').closest('[data-slot=badge]')).toHaveAttribute('title', 'Created by an agent');
  });
});

describe('the pickup hold', () => {
  it('says why auto-start passed a ticket over for overlapping work', () => {
    const { rerender } = render(<TicketBadges ticket={summary(1, { stage: 'todo', pickupHold: 'overlaps PDT-4 in api' })} />);
    expect(screen.getByText('Waiting: overlaps PDT-4 in api')).toBeInTheDocument();
    rerender(<TicketBadges ticket={summary(1, { stage: 'todo' })} />);
    expect(screen.queryByText(/Waiting:/)).not.toBeInTheDocument();
  });
});

describe('paused pickup', () => {
  it('says pickup is paused, with why in its title', () => {
    render(<TicketBadges ticket={summary(1, { stage: 'todo', pickupPaused: 'the disk is almost full (3.0 GB free)' })} />);
    expect(screen.getByText('Pickup paused').closest('[title]')).toHaveAttribute('title', expect.stringContaining('the disk is almost full'));
  });
});

describe('a failed auto-start', () => {
  it('says auto-start failed, with the error in its title', () => {
    const { rerender } = render(<TicketBadges ticket={summary(1, { stage: 'todo', pickupFailed: 'git failed: no repo' })} />);
    expect(screen.getByText('Auto-start failed').closest('[title]')).toHaveAttribute('title', expect.stringContaining('git failed: no repo'));
    rerender(<TicketBadges ticket={summary(1, { stage: 'todo' })} />);
    expect(screen.queryByText('Auto-start failed')).not.toBeInTheDocument();
  });
});
