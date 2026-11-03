import { render, screen, within } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { ViewHeader } from './ViewHeader';

describe('ViewHeader', () => {
  it('shows status, a heading title, meta and actions in order', () => {
    render(
      <ViewHeader
        leading={<span role="img" aria-label="running" />}
        title="fix login"
        titleTooltip="fix login (claude)"
        meta={<span>~/work/pdt</span>}
        actions={<><button type="button">Stop</button><button type="button" aria-label="More actions">⋯</button></>}
      />,
    );
    const header = screen.getByRole('banner');
    const heading = within(header).getByRole('heading', { name: 'fix login' });
    expect(heading).toHaveAttribute('title', 'fix login (claude)');
    expect(heading.className).toContain('truncate');
    expect(within(header).getByRole('img', { name: 'running' })).toBeInTheDocument();
    expect(within(header).getByText('~/work/pdt')).toBeInTheDocument();
    const buttons = within(header).getAllByRole('button').map((b) => b.getAttribute('aria-label') ?? b.textContent);
    expect(buttons).toEqual(['Stop', 'More actions']);
  });

  it('wraps on narrow screens and keeps a fixed minimum height', () => {
    render(<ViewHeader title="Run" />);
    const header = screen.getByRole('banner');
    expect(header.className).toContain('flex-wrap');
    expect(header.className).toContain('min-h-10');
  });

  it('leaves out empty slots and appends className', () => {
    render(<ViewHeader title="Only a title" className="bg-card" />);
    const header = screen.getByRole('banner');
    expect(within(header).queryByRole('button')).toBeNull();
    expect(header.className).toMatch(/bg-card$/);
  });

  it("pads tight by default, and with the page gutter when the view's content uses it", () => {
    const { rerender } = render(<ViewHeader title="Run" />);
    expect(screen.getByRole('banner').className).toContain('px-3');
    rerender(<ViewHeader title="Ticket" gutter />);
    const header = screen.getByRole('banner');
    expect(header.className).toContain('px-gutter');
    expect(header.className).not.toContain('px-3');
  });
});
