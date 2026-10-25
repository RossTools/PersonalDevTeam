import { act, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import { isAppPath, navigate, useAppLinks, useLocation, useNavigationGuard } from './router';

function Where() {
  useAppLinks();
  return <><p data-testid="loc">{useLocation()}</p><a href="/inbox/i1">item</a><a href="/api/health">api</a><a href="/pm" target="_blank">pm</a></>;
}

function Guarded({ answer }: { answer: { ok: boolean; asked: number } }) {
  useNavigationGuard(true, async () => { answer.asked++; return answer.ok; });
  return <p data-testid="loc">{useLocation()}</p>;
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
  it('asks a guard (unsaved edits) before navigating, and holds the page while it is set', async () => {
    const answer = { ok: false, asked: 0 };
    const { unmount } = render(<Guarded answer={answer} />);
    const before = window.location.pathname;
    await act(async () => navigate('/guarded'));
    expect(answer.asked).toBe(1);
    expect(window.location.pathname).toBe(before);
    const unload = new Event('beforeunload', { cancelable: true });
    window.dispatchEvent(unload);
    expect(unload.defaultPrevented).toBe(true);
    answer.ok = true;
    await act(async () => navigate('/guarded'));
    expect(screen.getByTestId('loc')).toHaveTextContent('/guarded');
    unmount();
    // Gone with the component.
    await act(async () => navigate('/free'));
    expect(answer.asked).toBe(2);
    expect(window.location.pathname).toBe('/free');
  });

  it('holds Back and Forward while a guard is set, and takes the step once it says yes', async () => {
    act(() => navigate('/one'));
    await new Promise((r) => setTimeout(r, 0));
    act(() => navigate('/two'));
    await new Promise((r) => setTimeout(r, 0));
    const answer = { ok: false, asked: 0 };
    const { unmount } = render(<Guarded answer={answer} />);
    const length = window.history.length;
    await act(() => window.history.back());
    await waitFor(() => expect(answer.asked).toBe(1));
    // Refused: still on the page, the history unchanged.
    await waitFor(() => expect(window.location.pathname).toBe('/two'));
    expect(screen.getByTestId('loc')).toHaveTextContent('/two');
    expect(window.history.length).toBe(length);
    answer.ok = true;
    await act(() => window.history.back());
    await waitFor(() => expect(screen.getByTestId('loc')).toHaveTextContent('/one'));
    expect(answer.asked).toBe(2);
    // Forward is held the same way.
    answer.ok = false;
    await act(() => window.history.forward());
    await waitFor(() => expect(answer.asked).toBe(3));
    await new Promise((r) => setTimeout(r, 20));
    expect(screen.getByTestId('loc')).toHaveTextContent('/one');
    unmount();
  });
});
