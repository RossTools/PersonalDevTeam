import { screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { renderWithQuery, stubApi } from '../test-utils';
import { GoogleSignIn, redirectUri } from './google';

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe('Google sign-in', () => {
  it('signs an account in through Google’s page in a window of its own, and signs another out', async () => {
    const popup = { location: { href: '' }, close: vi.fn() };
    const open = vi.spyOn(window, 'open').mockReturnValue(popup as unknown as Window);
    const calls = stubApi({
      'GET /api/ui/token': { token: 'ui' },
      'GET /api/google/client': { builtIn: false },
      'GET /api/google/accounts': [
        { connectionId: 'c1', name: 'Work', signedIn: false, email: null, hasClient: true, builtIn: false, agents: false, picker: false },
        { connectionId: 'c2', name: 'Home', signedIn: true, email: 'me@home.com', hasClient: true, builtIn: false, agents: true, picker: true },
      ],
      'POST /api/google/accounts/c1/sign-in': { url: 'https://accounts.google.com/o/oauth2/v2/auth?x=1', redirectUri: 'x' },
      'DELETE /api/google/accounts/c2/sign-in': undefined,
    });
    renderWithQuery(<GoogleSignIn />);
    expect(await screen.findByText('Signed in as me@home.com')).toBeInTheDocument();
    expect(screen.getByText(redirectUri())).toBeInTheDocument();
    expect(screen.getByText(/No file picker/)).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Allow agents' })).toBeNull();
    await userEvent.click(screen.getByRole('button', { name: 'Sign in' }));
    await waitFor(() => expect(popup.location.href).toBe('https://accounts.google.com/o/oauth2/v2/auth?x=1'));
    expect(open).toHaveBeenCalled();
    expect(calls.find((c) => c.path === '/api/google/accounts/c1/sign-in')?.body).toEqual({ origin: window.location.origin });
    expect(await screen.findByRole('status')).toHaveTextContent('Finish signing in');
    await userEvent.click(screen.getByRole('button', { name: 'Sign out' }));
    await waitFor(() => expect(calls.some((c) => c.method === 'DELETE' && c.path === '/api/google/accounts/c2/sign-in')).toBe(true));
  });

  it('adds an account with PDT’s own sign-in, and lets agents in and out of each', async () => {
    const popup = { location: { href: '' }, close: vi.fn() };
    vi.spyOn(window, 'open').mockReturnValue(popup as unknown as Window);
    const calls = stubApi({
      'GET /api/ui/token': { token: 'ui' },
      'GET /api/google/client': { builtIn: true },
      'GET /api/google/accounts': [
        { connectionId: 'c1', name: 'me@work.com', signedIn: true, email: 'me@work.com', hasClient: true, builtIn: true, agents: false, picker: true },
        { connectionId: 'c2', name: 'me@home.com', signedIn: true, email: 'me@home.com', hasClient: true, builtIn: true, agents: true, picker: true },
      ],
      'POST /api/google/sign-in': { url: 'https://accounts.google.com/o/oauth2/v2/auth?new=1', redirectUri: 'https://pdt.dev/oauth/google' },
      'POST /api/google/accounts/c1/sign-in': { url: 'https://accounts.google.com/o/oauth2/v2/auth?agent=1', redirectUri: 'https://pdt.dev/oauth/google' },
      'DELETE /api/google/accounts/c2/agents': undefined,
    });
    renderWithQuery(<GoogleSignIn />);
    expect(await screen.findByText('Agents allowed')).toBeInTheDocument();
    // Nothing to set up: no redirect URI to add, except under Advanced.
    expect(screen.getByText('Advanced')).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Sign in with Google' }));
    await waitFor(() => expect(popup.location.href).toBe('https://accounts.google.com/o/oauth2/v2/auth?new=1'));
    expect(calls.find((c) => c.path === '/api/google/sign-in')?.body).toEqual({ origin: window.location.origin });
    await userEvent.click(screen.getByRole('button', { name: 'Allow agents' }));
    await waitFor(() => expect(popup.location.href).toBe('https://accounts.google.com/o/oauth2/v2/auth?agent=1'));
    expect(calls.find((c) => c.path === '/api/google/accounts/c1/sign-in')?.body).toEqual({ origin: window.location.origin, agent: true });
    await userEvent.click(screen.getByRole('button', { name: 'Stop agents' }));
    await waitFor(() => expect(calls.some((c) => c.method === 'DELETE' && c.path === '/api/google/accounts/c2/agents')).toBe(true));
  });
});
