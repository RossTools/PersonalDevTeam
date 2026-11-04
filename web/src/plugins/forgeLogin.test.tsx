import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { forgeAccount, githubLogin as login, renderWithQuery, Reply, stubApi } from '../test-utils';
import { ForgeLogin } from './ForgeLogin';

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

const LOGIN = '/api/forges/github/login';
const ui = () => renderWithQuery(<ForgeLogin kind="github" label="GitHub" />);

describe('ForgeLogin', () => {
  it('says what the forge calls its change requests', async () => {
    stubApi({ 'GET /api/forges/hub/login': login({ kind: 'hub', label: 'Hub', changeRequestName: 'merge request' }) });
    renderWithQuery(<ForgeLogin kind="hub" label="Hub" />);
    expect(await screen.findByText(/PDT can’t follow merge requests or CI/)).toBeInTheDocument();
    expect(screen.getByText(/The logins PDT reads merge requests, CI and issues with/)).toBeInTheDocument();
  });

  it('connects with a code entered on GitHub', async () => {
    let entered = false;
    const pending = { userCode: 'ABCD-1234', verificationUri: 'https://github.com/login/device', expiresAt: 1, apiUrl: 'https://api.github.com' };
    let started = false;
    const calls = stubApi({
      [`GET ${LOGIN}`]: () => (entered ? login({ accounts: [forgeAccount('alex')] }) : login(started ? { pending } : {})),
      [`POST ${LOGIN}/device`]: () => { started = true; return login({ pending }); },
    });
    ui();
    expect(await screen.findByText('GitHub is not connected.')).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Connect GitHub' }));
    expect(await screen.findByText('ABCD-1234')).toBeInTheDocument();
    expect(screen.getByRole('link', { name: /github.com\/login\/device/ })).toHaveAttribute('href', 'https://github.com/login/device');
    expect(calls.filter((c) => c.method === 'POST')[0].body).toEqual({});
    entered = true;
    expect(await screen.findByText('alex', {})).toBeInTheDocument();
  });

  it('says which host no longer accepts its login, and reconnects', async () => {
    let rejected = true;
    const calls = stubApi({
      [`GET ${LOGIN}`]: () => login({ accounts: [forgeAccount('alex', 'github.com', { rejectedAt: rejected ? 1 : null })] }),
      [`POST ${LOGIN}/device`]: () => { rejected = false; return login({ accounts: [forgeAccount('alex')], pending: { userCode: 'ABCD-1234', verificationUri: 'https://github.com/login/device', expiresAt: 1, apiUrl: 'https://api.github.com' } }); },
    });
    ui();
    expect(await screen.findByText('GitHub on github.com no longer accepts this login.')).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Reconnect' }));
    expect(await screen.findByText('ABCD-1234')).toBeInTheDocument();
    expect(screen.queryByText('GitHub on github.com no longer accepts this login.')).toBeNull();
    expect(calls.some((c) => c.method === 'POST' && c.path === `${LOGIN}/device`)).toBe(true);
  });

  it('says when a token expires, and whether PDT renews it', async () => {
    const at = Date.UTC(2026, 9, 29, 21, 21);
    stubApi({
      [`GET ${LOGIN}`]: login({ accounts: [
        forgeAccount('alex', 'github.com'),
        forgeAccount('alexk', 'ghe.example.com', { expiresAt: at, renews: true }),
      ] }),
    });
    ui();
    expect(await screen.findByText(/On ghe\.example\.com\. Its token expires .+; PDT renews it before then\./)).toBeInTheDocument();
    expect(screen.getByText('On github.com.')).toBeInTheDocument();
  });

  it('tests a pasted token, saves it, and says why GitHub refused one', async () => {
    let n = 0;
    const calls = stubApi({
      [`GET ${LOGIN}`]: login({ deviceFlow: false }),
      [`POST ${LOGIN}/test`]: { host: 'github.com', apiUrl: 'https://api.github.com', webUrl: 'https://github.com', login: 'alex', scopes: ['repo'] },
      [`POST ${LOGIN}/token`]: () => (++n === 1
        ? new Reply(400, { code: 'invalid_token', message: 'GitHub does not accept this token' })
        : login({ accounts: [forgeAccount('alex')] })),
    });
    ui();
    await screen.findByText('GitHub is not connected.');
    expect(screen.queryByRole('button', { name: 'Connect GitHub' })).toBeNull();
    expect(screen.getByLabelText('GitHub API URL')).toHaveValue('https://api.github.com');
    const field = screen.getByLabelText('GitHub login token');
    await userEvent.type(field, 'bad');
    await userEvent.click(screen.getByRole('button', { name: 'Save login token' }));
    expect(await screen.findByText('GitHub does not accept this token')).toBeInTheDocument();
    await userEvent.clear(field);
    await userEvent.type(field, 'gho_good');
    await userEvent.click(screen.getByRole('button', { name: 'Test' }));
    expect(await screen.findByText('Signs in as alex on github.com.')).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Save login token' }));
    expect(await screen.findByText('alex')).toBeInTheDocument();
    await waitFor(() => expect(calls.filter((c) => c.method === 'POST').map((c) => c.body)).toEqual([{ token: 'bad' }, { token: 'gho_good' }, { token: 'gho_good' }]));
  });

  it('lists an account per host and adds an Enterprise Server beside github.com', async () => {
    let accounts = [forgeAccount('alex')];
    const calls = stubApi({
      [`GET ${LOGIN}`]: () => login({ accounts }),
      [`POST ${LOGIN}/token`]: () => { accounts = [...accounts, forgeAccount('alexk', 'ghe.example.com')]; return login({ accounts }); },
      [`DELETE ${LOGIN}/accounts/ghe.example.com`]: () => { accounts = accounts.slice(0, 1); return login({ accounts }); },
    });
    ui();
    await screen.findByText('alex');
    const url = screen.getByLabelText('GitHub API URL');
    await userEvent.clear(url);
    await userEvent.type(url, 'https://ghe.example.com/api/v3');
    await userEvent.type(screen.getByLabelText('GitHub login token'), 'ghp_x');
    await userEvent.click(screen.getByRole('button', { name: 'Save login token' }));
    expect(await screen.findByText('alexk')).toBeInTheDocument();
    expect(screen.getByText('alex')).toBeInTheDocument();
    expect(calls.find((c) => c.method === 'POST')?.body).toEqual({ token: 'ghp_x', apiUrl: 'https://ghe.example.com/api/v3' });
    await userEvent.click(screen.getByRole('button', { name: 'Disconnect ghe.example.com' }));
    await userEvent.click(within(await screen.findByRole('alertdialog')).getByRole('button', { name: 'Disconnect' }));
    await waitFor(() => expect(screen.queryByText('alexk')).toBeNull());
    expect(screen.getByText('alex')).toBeInTheDocument();
  });

  it('connects an Enterprise host with a code through its own OAuth App', async () => {
    const calls = stubApi({
      [`GET ${LOGIN}`]: login({ deviceFlow: false }),
      [`POST ${LOGIN}/device`]: login({ pending: { userCode: 'X', verificationUri: 'https://ghe/login/device', expiresAt: 1, apiUrl: 'https://ghe/api/v3' } }),
    });
    ui();
    const url = await screen.findByLabelText('GitHub API URL');
    await userEvent.clear(url);
    await userEvent.type(url, 'https://ghe/api/v3');
    await userEvent.type(screen.getByRole('textbox', { name: 'OAuth App client ID' }), 'Iv1.ghe');
    await userEvent.click(screen.getByRole('button', { name: 'Connect GitHub' }));
    await waitFor(() => expect(calls.find((c) => c.method === 'POST')?.body).toEqual({ apiUrl: 'https://ghe/api/v3', clientId: 'Iv1.ghe' }));
  });
});
