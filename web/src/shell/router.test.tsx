import { act, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import { isAppPath, navigate, useAppLinks, useLocation } from './router';

function Where() {
  useAppLinks();
  return <><p data-testid="loc">{useLocation()}</p><a href="/inbox/i1">item</a><a href="/api/health">api</a><a href="/pm" target="_blank">pm</a></>;
}

describe('router', () => {
  it('pushes and replaces history entries, and follows Back', async () => {
    render(<Where />);
    const start = window.history.length;
    act(() => navigate('/tickets'));
    await new Promise((r) => setTimeout(r, 0));
    act(() => navigate('/tickets?view=board'));
    expect(screen.getByTestId('loc')).toHaveTextContent('/tickets?view=board');
    expect(window.history.length).toBe(start + 2);
    act(() => navigate('/tickets', { replace: true }));
    expect(window.history.length).toBe(start + 2);
    await act(() => window.history.back());
    await screen.findByText('/tickets');
  });

  it('makes navigations in one go a single entry', async () => {
    render(<Where />);
    act(() => { navigate('/sessions'); navigate('/sessions/w/t'); });
    expect(screen.getByTestId('loc')).toHaveTextContent('/sessions/w/t');
    // Back goes past both, to where it started.
    await act(() => window.history.back());
    await screen.findByText('/');
  });

  it("navigates on plain clicks on the app's own links only", async () => {
    render(<Where />);
    await userEvent.click(screen.getByRole('link', { name: 'item' }));
    expect(screen.getByTestId('loc')).toHaveTextContent('/inbox/i1');
    expect(isAppPath('/api/health')).toBe(false);
    expect(isAppPath('/api')).toBe(false);
    expect(isAppPath('/apiary')).toBe(true);
    expect(isAppPath('//evil.test/x')).toBe(false);
    expect(isAppPath('https://x.test/')).toBe(false);
  });
});
