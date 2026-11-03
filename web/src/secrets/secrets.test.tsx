import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { fullText, renderWithQuery, stubApi } from '../test-utils';
import type { Secret } from '../api/gen/Secret';
import { SecretsPage } from './SecretsPage';

const secret = (name: string, over: Partial<Secret> = {}): Secret => ({
  id: `s-${name}`, name, description: '', projects: [], personas: [], createdAt: 0, updatedAt: 0,
  lastUsedAt: null, lastUsedBy: null, ...over,
});

describe('secrets', () => {
  afterEach(() => vi.unstubAllGlobals());

  it('lists names, who may use them and their last use, and shows a secret\'s uses', async () => {
    stubApi({
      'GET /api/secrets': [
        secret('NPM_TOKEN', { projects: ['p1'], description: 'Publishing', lastUsedAt: Date.now() - 60_000, lastUsedBy: 'Coder · PDT-9' }),
        secret('API'),
      ],
      'GET /api/projects': [{ id: 'p1', name: 'Shop', kind: 'repo' }],
      'GET /api/personas': [],
      'GET /api/secrets/store': { backend: 'keychain', path: null, problem: null },
      'GET /api/secrets/s-NPM_TOKEN/uses': [
        { id: 1, secretId: 's-NPM_TOKEN', at: Date.now() - 60_000, by: 'Coder · PDT-9', how: 'run_with_secrets', sessionId: 'x', runId: null },
      ],
    });
    renderWithQuery(<SecretsPage />);
    expect(await screen.findByText('NPM_TOKEN')).toBeInTheDocument();
    expect(await screen.findByText('Secrets: system keyring')).toBeInTheDocument();
    expect(await screen.findByText('Shop · every agent')).toBeInTheDocument();
    expect(screen.getByText('Every project · every agent')).toBeInTheDocument();
    expect(screen.getByText(fullText(/^Used 1m ago by Coder · PDT-9$/))).toBeInTheDocument();
    expect(screen.getByText('Not used yet')).toBeInTheDocument();
    await userEvent.click(screen.getAllByRole('button', { name: 'Uses' })[0]);
    const uses = await screen.findByRole('list', { name: 'Uses' });
    expect(within(uses).getByText('run_with_secrets')).toBeInTheDocument();
  });

  it('says when values are kept in a file in the data folder', async () => {
    stubApi({
      'GET /api/secrets': [],
      'GET /api/projects': [],
      'GET /api/personas': [],
      'GET /api/secrets/store': { backend: 'file', path: '/home/me/.local/share/pdt/secrets.json', problem: null },
    });
    renderWithQuery(<SecretsPage />);
    expect(await screen.findByText(/Secrets: file in the data folder/)).toBeInTheDocument();
    expect(screen.getByText('/home/me/.local/share/pdt/secrets.json')).toBeInTheDocument();
  });

  it('says when the keyring this machine uses does not answer', async () => {
    stubApi({
      'GET /api/secrets': [],
      'GET /api/projects': [],
      'GET /api/personas': [],
      'GET /api/secrets/store': { backend: 'keychain', path: null, problem: 'The Secret Service did not answer at startup.' },
    });
    renderWithQuery(<SecretsPage />);
    expect(await screen.findByText('Secrets: system keyring')).toBeInTheDocument();
    expect(screen.getByText('The Secret Service did not answer at startup.')).toBeInTheDocument();
  });

  it('adds a secret with a masked value, limited to the projects ticked', async () => {
    const calls = stubApi({
      'GET /api/secrets': [],
      'GET /api/projects': [{ id: 'p1', name: 'Shop', kind: 'repo' }, { id: 'h', name: 'Host', kind: 'host' }],
      'GET /api/personas': [{ id: 'c', name: 'Coder', archived: false }],
      'POST /api/secrets': secret('NPM_TOKEN'),
    });
    renderWithQuery(<SecretsPage />);
    await userEvent.click(await screen.findByRole('button', { name: 'Add secret' }));
    const dialog = await screen.findByRole('dialog');
    await userEvent.type(within(dialog).getByLabelText('Name'), 'NPM_TOKEN');
    const value = within(dialog).getByLabelText('Value');
    expect(value).toHaveAttribute('type', 'password');
    await userEvent.type(value, 'npm_secret_1');
    await userEvent.click(await within(dialog).findByText('Shop'));
    expect(within(dialog).getByText('The PM (personal workflows)')).toBeInTheDocument();
    await userEvent.click(within(dialog).getByRole('button', { name: 'Save' }));
    await waitFor(() => expect(calls.find((c) => c.method === 'POST')?.body).toEqual({
      name: 'NPM_TOKEN', description: '', projects: ['p1'], personas: [], value: 'npm_secret_1',
    }));
  });
});
