import { fireEvent, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { SignIn } from '../api/gen/SignIn';
import { renderWithQuery, stubApi } from '../test-utils';
import { connectionsKey } from '../connections/ConnectionDialog';
import { AccountSignIn, redirectUri } from './AccountSignIn';

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

const signIn = (builtIn: boolean): SignIn => ({
  api: '/api/acme',
  service: 'Acme',
  kind: 'acme',
  builtIn,
  description: 'Sign in with Acme adds an account.',
  ownClient: 'Blocked? Use your own client.',
});

describe('A plugin’s account sign-in', () => {
  it('signs an account in through the service’s page in a window of its own, and signs another out', async () => {
    const popup = { location: { href: '' }, close: vi.fn() };
    const open = vi.spyOn(window, 'open').mockReturnValue(popup as unknown as Window);
    const calls = stubApi({
      'GET /api/ui/token': { token: 'ui' },
      'GET /api/acme/accounts': [
        { connectionId: 'c1', name: 'Work', signedIn: false, email: null, hasClient: true, builtIn: false, agents: false, picker: false, note: 'No file picker (needs a key)' },
        { connectionId: 'c2', name: 'Home', signedIn: true, email: 'me@home.com', hasClient: true, builtIn: false, agents: true, picker: true, note: null },
      ],
      'POST /api/acme/accounts/c1/sign-in': { url: 'https://acme.example/consent?x=1', redirectUri: 'x' },
      'DELETE /api/acme/accounts/c2/sign-in': undefined,
    });
    renderWithQuery(<AccountSignIn signIn={signIn(false)} />);
    expect(await screen.findByText('Signed in as me@home.com')).toBeInTheDocument();
    expect(screen.getByText(redirectUri(signIn(false)))).toBeInTheDocument();
    expect(redirectUri(signIn(false))).toBe(`${window.location.origin}/api/acme/callback`);
    expect(screen.getByText('No file picker (needs a key)')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Allow agents' })).toBeNull();
    await userEvent.click(screen.getByRole('button', { name: 'Sign in' }));
    await waitFor(() => expect(popup.location.href).toBe('https://acme.example/consent?x=1'));
    expect(open).toHaveBeenCalled();
    expect(calls.find((c) => c.path === '/api/acme/accounts/c1/sign-in')?.body).toEqual({ origin: window.location.origin });
    expect(await screen.findByRole('status')).toHaveTextContent('Finish signing in in the Acme window');
    await userEvent.click(screen.getByRole('button', { name: 'Sign out' }));
    await waitFor(() => expect(calls.some((c) => c.method === 'DELETE' && c.path === '/api/acme/accounts/c2/sign-in')).toBe(true));
  });

  it('adds an account with PDT’s own client, and lets agents in and out of each', async () => {
    const popup = { location: { href: '' }, close: vi.fn() };
    vi.spyOn(window, 'open').mockReturnValue(popup as unknown as Window);
    const calls = stubApi({
      'GET /api/ui/token': { token: 'ui' },
      'GET /api/acme/accounts': [
        { connectionId: 'c1', name: 'me@work.com', signedIn: true, email: 'me@work.com', hasClient: true, builtIn: true, agents: false, picker: true, note: null },
        { connectionId: 'c2', name: 'me@home.com', signedIn: true, email: 'me@home.com', hasClient: true, builtIn: true, agents: true, picker: true, note: null },
      ],
      'POST /api/acme/sign-in': { url: 'https://acme.example/consent?new=1', redirectUri: 'https://pdt.dev/oauth/acme' },
      'POST /api/acme/accounts/c1/sign-in': { url: 'https://acme.example/consent?agent=1', redirectUri: 'https://pdt.dev/oauth/acme' },
      'DELETE /api/acme/accounts/c2/agents': undefined,
    });
    renderWithQuery(<AccountSignIn signIn={signIn(true)} />);
    expect(await screen.findByText('Agents allowed')).toBeInTheDocument();
    expect(screen.getByText('Sign in with Acme adds an account.')).toBeInTheDocument();
    // Nothing to set up: no redirect URI to add, except under Advanced.
    expect(screen.getByText('Advanced')).toBeInTheDocument();
    expect(screen.getByText(/Blocked\? Use your own client\./)).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Sign in with Acme' }));
    await waitFor(() => expect(popup.location.href).toBe('https://acme.example/consent?new=1'));
    expect(calls.find((c) => c.path === '/api/acme/sign-in')?.body).toEqual({ origin: window.location.origin });
    await userEvent.click(screen.getByRole('button', { name: 'Allow agents' }));
    await waitFor(() => expect(popup.location.href).toBe('https://acme.example/consent?agent=1'));
    expect(calls.find((c) => c.path === '/api/acme/accounts/c1/sign-in')?.body).toEqual({ origin: window.location.origin, agent: true });
    await userEvent.click(screen.getByRole('button', { name: 'Stop agents' }));
    await waitFor(() => expect(calls.some((c) => c.method === 'DELETE' && c.path === '/api/acme/accounts/c2/agents')).toBe(true));
  });

  it('starts PDT’s own sign-in at the website’s page and hands it the consent page once it is listening', async () => {
    const popup = { location: { href: '' }, close: vi.fn(), postMessage: vi.fn() };
    vi.spyOn(window, 'open').mockReturnValue(popup as unknown as Window);
    stubApi({
      'GET /api/ui/token': { token: 'ui' },
      'GET /api/acme/accounts': [],
      'POST /api/acme/sign-in': { url: 'https://acme.example/consent?new=1', redirectUri: 'https://pdt.dev/oauth/acme', relay: 'https://pdt.dev/oauth/acme' },
    });
    renderWithQuery(<AccountSignIn signIn={signIn(true)} />);
    await userEvent.click(await screen.findByRole('button', { name: 'Sign in with Acme' }));
    await waitFor(() => expect(popup.location.href).toBe('https://pdt.dev/oauth/acme'));
    const ready = (origin: string, source: unknown) =>
      fireEvent(window, new MessageEvent('message', { data: { pdtRelay: 'ready' }, origin, source: source as Window }));
    // Only the relay page, in the sign-in window, gets it.
    ready('https://evil.example', popup);
    ready('https://pdt.dev', window);
    expect(popup.postMessage).not.toHaveBeenCalled();
    ready('https://pdt.dev', popup);
    expect(popup.postMessage).toHaveBeenCalledWith({ pdtRelayTo: 'https://acme.example/consent?new=1' }, 'https://pdt.dev');
    ready('https://pdt.dev', popup);
    expect(popup.postMessage).toHaveBeenCalledTimes(1);
  });

  it('asks for pop-ups when PDT’s own sign-in can’t open its window', async () => {
    vi.spyOn(window, 'open').mockReturnValue(null);
    const assign = vi.fn();
    vi.stubGlobal('location', { ...window.location, assign, origin: window.location.origin });
    stubApi({
      'GET /api/ui/token': { token: 'ui' },
      'GET /api/acme/accounts': [],
      'POST /api/acme/sign-in': { url: 'https://acme.example/consent?new=1', redirectUri: 'https://pdt.dev/oauth/acme', relay: 'https://pdt.dev/oauth/acme' },
    });
    renderWithQuery(<AccountSignIn signIn={signIn(true)} />);
    await userEvent.click(await screen.findByRole('button', { name: 'Sign in with Acme' }));
    expect(await screen.findByText(/Allow pop-ups for PDT/)).toBeInTheDocument();
    expect(assign).not.toHaveBeenCalled();
  });

  it('lists a connection as soon as it is saved, without a reload', async () => {
    let accounts: object[] = [];
    stubApi({
      'GET /api/ui/token': { token: 'ui' },
      'GET /api/acme/accounts': () => accounts,
    });
    const { qc } = renderWithQuery(<AccountSignIn signIn={signIn(false)} />);
    expect(await screen.findByText('Add a connection for your Acme account first.')).toBeInTheDocument();
    accounts = [{ connectionId: 'c1', name: 'Work', signedIn: false, email: null, hasClient: true, builtIn: false, agents: false, picker: false, note: null }];
    // What saving or deleting a connection does.
    await qc.invalidateQueries({ queryKey: connectionsKey });
    expect(await screen.findByText('Work')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Sign in' })).toBeEnabled();
  });
});
