import { screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { renderWithQuery, Reply, stubApi } from '../test-utils';
import { GithubLogin } from './GithubLogin';

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

const login = (over: object = {}) => ({
  connected: false, login: null, apiUrl: 'https://api.github.com', webUrl: 'https://github.com', deviceFlow: true,
  scopes: 'repo', pending: null, error: null, rejectedAt: null, ...over,
});

describe('GithubLogin', () => {
  it('connects with a code entered on GitHub', async () => {
    let entered = false;
    const pending = { userCode: 'ABCD-1234', verificationUri: 'https://github.com/login/device', expiresAt: 1 };
    let started = false;
    const calls = stubApi({
      'GET /api/github/login': () => (entered ? login({ connected: true, login: 'ross' }) : login(started ? { pending } : {})),
      'POST /api/github/login/device': () => { started = true; return login({ pending }); },
    });
    renderWithQuery(<GithubLogin />);
    expect(await screen.findByText('GitHub is not connected.')).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Connect GitHub' }));
    expect(await screen.findByText('ABCD-1234')).toBeInTheDocument();
    expect(screen.getByRole('link', { name: /github.com\/login\/device/ })).toHaveAttribute('href', 'https://github.com/login/device');
    expect(calls.filter((c) => c.method === 'POST')[0].body).toEqual({});
    entered = true;
    expect(await screen.findByText('ross', {}, { timeout: 4000 })).toBeInTheDocument();
  });

  it('says when GitHub no longer accepts the login, and reconnects', async () => {
    let rejected = true;
    const calls = stubApi({
      'GET /api/github/login': () => login({ connected: true, login: 'ross', rejectedAt: rejected ? 1 : null }),
      'POST /api/github/login/device': () => { rejected = false; return login({ connected: true, login: 'ross', pending: { userCode: 'ABCD-1234', verificationUri: 'https://github.com/login/device', expiresAt: 1 } }); },
    });
    renderWithQuery(<GithubLogin />);
    expect(await screen.findByText('GitHub no longer accepts this login.')).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Reconnect' }));
    expect(await screen.findByText('ABCD-1234')).toBeInTheDocument();
    expect(screen.queryByText('GitHub no longer accepts this login.')).toBeNull();
    expect(calls.some((c) => c.method === 'POST' && c.path === '/api/github/login/device')).toBe(true);
  });

  it('saves a pasted token, and says why GitHub refused one', async () => {
    let n = 0;
    const calls = stubApi({
      'GET /api/github/login': login({ deviceFlow: false }),
      'POST /api/github/login/token': () => (++n === 1
        ? new Reply(400, { code: 'invalid_token', message: 'GitHub does not accept this token' })
        : login({ connected: true, login: 'ross' })),
    });
    renderWithQuery(<GithubLogin />);
    await screen.findByText('GitHub is not connected.');
    expect(screen.queryByRole('button', { name: 'Connect GitHub' })).toBeNull();
    const field = screen.getByLabelText('GitHub login token');
    await userEvent.type(field, 'bad');
    await userEvent.click(screen.getByRole('button', { name: 'Save login token' }));
    expect(await screen.findByText('GitHub does not accept this token')).toBeInTheDocument();
    await userEvent.clear(field);
    await userEvent.type(field, 'gho_good');
    await userEvent.click(screen.getByRole('button', { name: 'Save login token' }));
    expect(await screen.findByText('ross')).toBeInTheDocument();
    await waitFor(() => expect(calls.filter((c) => c.method === 'POST').map((c) => c.body)).toEqual([{ token: 'bad' }, { token: 'gho_good' }]));
  });

  it('signs in to an Enterprise host by its API URL', async () => {
    const calls = stubApi({
      'GET /api/github/login': login({ deviceFlow: false }),
      'POST /api/github/login/device': login({ pending: { userCode: 'X', verificationUri: 'https://ghe/login/device', expiresAt: 1 } }),
    });
    renderWithQuery(<GithubLogin />);
    await userEvent.click(await screen.findByRole('button', { name: 'Use an Enterprise host' }));
    await userEvent.type(screen.getByLabelText('GitHub API URL'), 'https://ghe/api/v3');
    await userEvent.type(screen.getByLabelText('Enterprise OAuth App client ID'), 'Iv1.ghe');
    await userEvent.click(screen.getByRole('button', { name: 'Connect GitHub' }));
    await waitFor(() => expect(calls.find((c) => c.method === 'POST')?.body).toEqual({ apiUrl: 'https://ghe/api/v3', clientId: 'Iv1.ghe' }));
  });
});
