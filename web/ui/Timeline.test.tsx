import { render, screen, within } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { RelativeTime, Timeline, type TimelineItem } from './Timeline';
import { absoluteTime } from './lib/time';

const NOW = new Date(2026, 9, 17, 12, 0).getTime();
const H = 3_600_000;

describe('Timeline', () => {
  const items: TimelineItem[] = [
    { id: '1', actor: 'user', time: NOW - 2 * H, title: 'Created ticket' },
    { id: '2', actor: 'system', time: NOW - 5 * 60_000, title: 'Moved to Ready' },
    { id: '3', actor: 'reviewer', time: NOW - 30_000, title: 'Comment', body: <p>Looks good.</p> },
    { id: '4', actor: 'agent', time: NOW - 10_000, title: 'Comment' },
  ];

  it('lists entries with actor, relative time (absolute in the title) and body', () => {
    render(<Timeline items={items} now={NOW} />);
    const list = screen.getByRole('list', { name: 'Activity' });
    const rows = within(list).getAllByRole('listitem');
    expect(rows).toHaveLength(4);
    expect(rows[0]).toHaveTextContent('Created ticket');
    expect(rows[0]).toHaveTextContent('You · 2h ago');
    expect(rows[1]).toHaveTextContent('PDT · 5m ago');
    expect(rows[2]).toHaveTextContent('reviewer · just now');
    expect(within(rows[2]).getByText('Looks good.')).toBeInTheDocument();
    expect(rows[3]).toHaveTextContent('Agent · just now');
    const time = within(rows[0]).getByText('2h ago');
    expect(time.tagName).toBe('TIME');
    expect(time).toHaveAttribute('title', absoluteTime(NOW - 2 * H));
  });

  it('uses a custom icon when given', () => {
    render(<Timeline items={[{ ...items[0], icon: <span data-testid="icon" /> }]} now={NOW} />);
    expect(screen.getByTestId('icon')).toBeInTheDocument();
  });

  it('shows the empty state', () => {
    render(<Timeline items={[]} empty="Nothing happened yet." />);
    expect(screen.getByText('Nothing happened yet.')).toBeInTheDocument();
    expect(screen.queryByRole('list')).toBeNull();
  });
});

describe('RelativeTime', () => {
  it('shows a short age with short, the absolute time in its tooltip', () => {
    render(<RelativeTime time={NOW - 3 * H} now={NOW} short />);
    const time = screen.getByText('3h');
    expect(time).toHaveAttribute('title', absoluteTime(NOW - 3 * H));
  });
});
