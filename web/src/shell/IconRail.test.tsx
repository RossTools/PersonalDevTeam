import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it } from 'vitest';
import { IconRail, RAIL_EXPANDED_KEY } from './IconRail';

beforeEach(() => localStorage.clear());

const rail = () => screen.getByRole('navigation', { name: 'Views' });

describe('IconRail', () => {
  it('expands to show every view name beside its icon, and collapses again', async () => {
    render(<IconRail active="sessions" onSelect={() => {}} counts={{ inbox: 3 }} />);
    expect(within(rail()).queryByText('Tickets')).not.toBeInTheDocument();
    const expand = within(rail()).getByRole('button', { name: 'Expand side rail' });
    expect(expand).toHaveAttribute('aria-expanded', 'false');
    await userEvent.click(expand);
    for (const name of ['Project manager', 'Sessions', 'Inbox', 'Tickets', 'Notes', 'Workflows', 'Personas', 'Files', 'Settings']) {
      expect(within(rail()).getByText(name)).toBeInTheDocument();
    }
    // The badge still shows beside the label, and the item keeps its counted name.
    expect(within(rail()).getByRole('button', { name: 'Inbox (3)' })).toHaveTextContent('Inbox3');
    expect(within(rail()).getByRole('button', { name: 'Sessions' })).toHaveAttribute('aria-current', 'page');
    const collapse = within(rail()).getByRole('button', { name: 'Collapse side rail' });
    expect(collapse).toHaveAttribute('aria-expanded', 'true');
    await userEvent.click(collapse);
    expect(within(rail()).queryByText('Tickets')).not.toBeInTheDocument();
    expect(within(rail()).getByRole('button', { name: 'Inbox (3)' })).toHaveTextContent('3');
  });

  it('remembers the state in this browser across a reload', async () => {
    const { unmount } = render(<IconRail active="sessions" onSelect={() => {}} />);
    await userEvent.click(screen.getByRole('button', { name: 'Expand side rail' }));
    expect(localStorage.getItem(RAIL_EXPANDED_KEY)).toBe('true');
    unmount();
    render(<IconRail active="sessions" onSelect={() => {}} />);
    expect(screen.getByRole('button', { name: 'Collapse side rail' })).toBeInTheDocument();
    expect(within(rail()).getByText('Tickets')).toBeInTheDocument();
  });

  it('shows tooltips only while collapsed', async () => {
    const { unmount } = render(<IconRail active="sessions" onSelect={() => {}} />);
    // A keyboard focus opens a tooltip at once (no hover delay): one missing after the focus is not coming.
    await userEvent.tab();
    expect(screen.getByRole('tooltip')).toHaveAttribute('data-placement', 'right');
    unmount();
    localStorage.setItem(RAIL_EXPANDED_KEY, 'true');
    render(<IconRail active="sessions" onSelect={() => {}} />);
    await userEvent.tab();
    expect(screen.getByRole('button', { name: 'Project manager' })).toHaveFocus();
    expect(screen.queryByRole('tooltip')).toBeNull();
  });
});
