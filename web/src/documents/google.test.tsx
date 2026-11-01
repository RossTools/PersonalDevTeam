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
      'GET /api/google/accounts': [
        { connectionId: 'c1', name: 'Work', signedIn: false, email: null, hasClient: true, picker: false },
        { connectionId: 'c2', name: 'Home', signedIn: true, email: 'me@home.com', hasClient: true, picker: true },
      ],
      'POST /api/google/accounts/c1/sign-in': { url: 'https://accounts.google.com/o/oauth2/v2/auth?x=1', redirectUri: 'x' },
      'DELETE /api/google/accounts/c2/sign-in': undefined,
    });
    renderWithQuery(<GoogleSignIn />);
    expect(await screen.findByText('Signed in as me@home.com')).toBeInTheDocument();
    expect(screen.getByText(redirectUri())).toBeInTheDocument();
    expect(screen.getByText(/No file picker/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Sign in with Google' }));
    await waitFor(() => expect(popup.location.href).toBe('https://accounts.google.com/o/oauth2/v2/auth?x=1'));
    expect(open).toHaveBeenCalled();
    expect(calls.find((c) => c.path === '/api/google/accounts/c1/sign-in')?.body).toEqual({ origin: window.location.origin });
    expect(await screen.findByRole('status')).toHaveTextContent('Finish signing in');
    await userEvent.click(screen.getByRole('button', { name: 'Sign out' }));
    await waitFor(() => expect(calls.some((c) => c.method === 'DELETE' && c.path === '/api/google/accounts/c2/sign-in')).toBe(true));
  });
});
