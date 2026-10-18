import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useState } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { FolderListing } from '../api/gen/FolderListing';
import { AddProjectForm } from '../projects/AddProjectForm';
import { renderWithQuery, stubApi, type Call } from '../test-utils';
import { FolderField } from './FolderField';

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

const places = [{ name: 'Home', path: '/home/me' }, { name: '/', path: '/' }];
const work: FolderListing = {
  path: '/home/me/work', display: '~/work', parent: '/home/me', sep: '/',
  crumbs: [{ name: '~', path: '/home/me' }, { name: 'work', path: '/home/me/work' }],
  places,
  folders: [{ name: 'app', path: '/home/me/work/app', display: '~/work/app', hidden: false, git: true, project: null }],
  truncated: false,
};
const app: FolderListing = { ...work, path: '/home/me/work/app', display: '~/work/app', parent: '/home/me/work', folders: [] };

/** Answers GET /api/host/folders from the `path` query parameter. */
const folders = (c: Call) => {
  const path = new URL(c.path, 'http://x').searchParams.get('path') ?? '';
  return path === '/home/me/work/app' ? app : work;
};

function Controlled({ start }: { start?: string }) {
  const [v, setV] = useState('');
  return <FolderField aria-label="Folder" start={start} value={v} onChange={setV} />;
}

describe('FolderField', () => {
  it('opens at the start folder (the nearest that exists) and writes the chosen folder', async () => {
    const calls = stubApi({ 'GET /api/host/folders': folders });
    renderWithQuery(<Controlled start="~/work" />);
    await userEvent.click(screen.getByRole('button', { name: 'Browse for Folder' }));
    const dialog = await screen.findByRole('dialog', { name: 'Choose a folder' });
    await within(dialog).findByRole('row', { name: 'app' });
    expect(calls[0].path).toBe('/api/host/folders?path=%7E%2Fwork&nearest=true');
    await userEvent.click(within(dialog).getByRole('row', { name: 'app' }));
    await waitFor(() => expect(within(dialog).getByRole('textbox', { name: 'Folder path' })).toHaveValue('~/work/app/'));
    expect(calls.at(-1)!.path).toBe('/api/host/folders?path=%2Fhome%2Fme%2Fwork%2Fapp');
    await userEvent.click(within(dialog).getByRole('button', { name: 'Choose' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    expect(screen.getByRole('textbox', { name: 'Folder' })).toHaveValue('~/work/app');
  });

  it('starts from what the field holds', async () => {
    const calls = stubApi({ 'GET /api/host/folders': folders });
    renderWithQuery(<Controlled />);
    await userEvent.type(screen.getByRole('textbox', { name: 'Folder' }), '/home/me/work/new');
    await userEvent.click(screen.getByRole('button', { name: 'Browse for Folder' }));
    await screen.findByRole('row', { name: 'app' });
    expect(calls[0].path).toBe('/api/host/folders?path=%2Fhome%2Fme%2Fwork%2Fnew&nearest=true');
  });

  it('makes a new folder in the one on screen', async () => {
    const calls = stubApi({
      'GET /api/host/folders': folders,
      'POST /api/host/folders': { name: 'fresh', path: '/home/me/work/fresh', display: '~/work/fresh', hidden: false, git: false, project: null },
    });
    renderWithQuery(<Controlled start="~/work" />);
    await userEvent.click(screen.getByRole('button', { name: 'Browse for Folder' }));
    await screen.findByRole('row', { name: 'app' });
    await userEvent.click(screen.getByRole('button', { name: 'New folder' }));
    await userEvent.type(screen.getByRole('textbox', { name: 'New folder name' }), 'fresh{Enter}');
    await waitFor(() => expect(calls).toContainEqual({ method: 'POST', path: '/api/host/folders', body: { parent: '/home/me/work', name: 'fresh' } }));
  });

  it('fills Add project’s path and name from the picked repository', async () => {
    stubApi({
      'GET /api/host/settings': { settings: {}, projectsDir: '/home/me/work', ownerFromEnv: null, tokenPresent: false, tokenError: null, remote: {} },
      'GET /api/host/folders': folders,
    });
    renderWithQuery(<AddProjectForm />);
    await userEvent.click(screen.getByRole('button', { name: 'Browse folders' }));
    const dialog = await screen.findByRole('dialog', { name: 'Choose the repository' });
    await userEvent.click(await within(dialog).findByRole('row', { name: 'app' }));
    await waitFor(() => expect(within(dialog).getByRole('textbox', { name: 'Folder path' })).toHaveValue('~/work/app/'));
    await userEvent.click(within(dialog).getByRole('button', { name: 'Choose' }));
    await waitFor(() => expect(screen.getByRole('textbox', { name: 'Repository path' })).toHaveValue('~/work/app'));
    expect(screen.getByRole('textbox', { name: 'Name' })).toHaveValue('app');
  });
});
