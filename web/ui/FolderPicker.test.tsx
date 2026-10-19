import { act, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useState } from 'react';
import { describe, expect, it, vi } from 'vitest';
import { FolderPicker, splitTyped, type FolderPickerFolder, type FolderPickerListing, type FolderPickerSelection } from './FolderPicker';

const folder = (path: string, display: string, extra: Partial<FolderPickerFolder> = {}): FolderPickerFolder => ({
  name: path.split(/[\\/]/).pop()!,
  path,
  display,
  ...extra,
});

/** A small macOS host: /Users/me (home) with work/{app, api, notes}. */
const mac: Record<string, FolderPickerListing> = {
  '/Users/me': {
    path: '/Users/me', display: '~', parent: '/Users', sep: '/',
    crumbs: [{ name: '~', path: '/Users/me' }],
    places: [{ name: 'Home', path: '/Users/me' }, { name: '/', path: '/' }],
    folders: [folder('/Users/me/work', '~/work')],
  },
  '/Users/me/work': {
    path: '/Users/me/work', display: '~/work', parent: '/Users/me', sep: '/',
    crumbs: [{ name: '~', path: '/Users/me' }, { name: 'work', path: '/Users/me/work' }],
    places: [{ name: 'Home', path: '/Users/me' }, { name: '/', path: '/' }],
    folders: [
      folder('/Users/me/work/api', '~/work/api', { git: true }),
      folder('/Users/me/work/app', '~/work/app', { git: true, project: { name: 'App', key: 'APP' } }),
      folder('/Users/me/work/notes', '~/work/notes'),
    ],
  },
};
const aliases: Record<string, string> = { '~': '/Users/me', '~/': '/Users/me', '~/work': '/Users/me/work', '~/work/': '/Users/me/work' };

/** The picker wired to a fake host, like FolderField wires it to the API. */
function Harness({
  host = mac,
  alias = aliases,
  start,
  onBrowse,
  onSelection,
  onCreate,
}: {
  host?: Record<string, FolderPickerListing>;
  alias?: Record<string, string>;
  start: string;
  onBrowse?: (p: string) => void;
  onSelection?: (s: FolderPickerSelection | null) => void;
  onCreate?: (name: string) => Promise<FolderPickerFolder>;
}) {
  const [path, setPath] = useState(start);
  const [hidden, setHidden] = useState(false);
  const real = alias[path] ?? path;
  const listing = host[real];
  return (
    <FolderPicker
      listing={listing ?? host[start]}
      error={listing ? null : `${path} does not exist`}
      onBrowse={(p) => { onBrowse?.(p); setPath(p); }}
      showHidden={hidden}
      onShowHiddenChange={setHidden}
      onSelectionChange={onSelection}
      onCreateFolder={onCreate}
    />
  );
}

describe('splitTyped', () => {
  it('splits at the last separator, accepting / on Windows', () => {
    expect(splitTyped('~/work/ap', '/')).toEqual({ dir: '~/work/', leaf: 'ap' });
    expect(splitTyped('~', '/')).toEqual({ dir: '', leaf: '~' });
    expect(splitTyped('C:\\Users\\me', '\\')).toEqual({ dir: 'C:\\Users\\', leaf: 'me' });
    expect(splitTyped('C:/Users/me', '\\')).toEqual({ dir: 'C:/Users/', leaf: 'me' });
    expect(splitTyped('/a\\b', '/')).toEqual({ dir: '/', leaf: 'a\\b' });
  });
});

describe('FolderPicker', () => {
  it('shows the breadcrumb, places and folders, marking git repos and projects', () => {
    render(<Harness start="/Users/me/work" />);
    const crumbs = within(screen.getByRole('navigation', { name: 'Folder path' }));
    expect(crumbs.getByRole('button', { name: '~' })).toBeInTheDocument();
    expect(crumbs.getByRole('button', { name: 'work' })).toHaveAttribute('aria-current', 'location');
    expect(within(screen.getByRole('group', { name: 'Places' })).getByRole('button', { name: 'Home' })).toBeInTheDocument();
    expect(screen.getByRole('textbox', { name: 'Folder path' })).toHaveValue('~/work/');
    const app = screen.getByRole('row', { name: 'app' });
    expect(within(app).getByText('APP')).toBeInTheDocument();
    expect(within(screen.getByRole('row', { name: 'api' })).getByText('git')).toBeInTheDocument();
    expect(within(screen.getByRole('row', { name: 'notes' })).queryByText('git')).not.toBeInTheDocument();
  });

  it('opens a folder on click and goes up', async () => {
    const browse = vi.fn();
    render(<Harness start="/Users/me" onBrowse={browse} />);
    await userEvent.click(screen.getByRole('row', { name: 'work' }));
    expect(browse).toHaveBeenLastCalledWith('/Users/me/work');
    await waitFor(() => expect(screen.getByRole('textbox', { name: 'Folder path' })).toHaveValue('~/work/'));
    await userEvent.click(screen.getByRole('button', { name: 'Up one folder' }));
    expect(browse).toHaveBeenLastCalledWith('/Users/me');
  });

  it('filters as you type, completes with Tab and reports the exact match', async () => {
    const sel = vi.fn();
    render(<Harness start="/Users/me/work" onSelection={sel} />);
    const field = screen.getByRole('textbox', { name: 'Folder path' });
    expect(sel).toHaveBeenLastCalledWith({ path: '/Users/me/work', display: '~/work', name: 'work' });
    await userEvent.type(field, 'a');
    expect(screen.getAllByRole('row').map((r) => r.textContent)).toEqual(['apigit', 'appAPP']);
    await userEvent.type(field, 'pp');
    expect(sel).toHaveBeenLastCalledWith({ path: '/Users/me/work/app', display: '~/work/app', name: 'app' });
    await userEvent.clear(field);
    await userEvent.type(field, '~/work/n');
    // Tab completes from the folder the text names, once it has loaded.
    await waitFor(() => expect(screen.getAllByRole('row').map((r) => r.textContent)).toEqual(['notes']));
    await userEvent.type(field, '{Tab}');
    expect(field).toHaveValue('~/work/notes/');
  });

  it('lists the folder a typed separator ends', async () => {
    const browse = vi.fn();
    render(<Harness start="/Users/me/work" onBrowse={browse} />);
    const field = screen.getByRole('textbox', { name: 'Folder path' });
    await userEvent.clear(field);
    await userEvent.type(field, '~/');
    await waitFor(() => expect(browse).toHaveBeenLastCalledWith('~/'));
    expect(field).toHaveValue('~/'); // typing is never overwritten by the listing
    expect(screen.getByRole('row', { name: 'work' })).toBeInTheDocument();
  });

  it('goes to a typed path on Enter and shows why it failed', async () => {
    const browse = vi.fn();
    render(<Harness start="/Users/me" onBrowse={browse} />);
    const field = screen.getByRole('textbox', { name: 'Folder path' });
    await userEvent.clear(field);
    await userEvent.type(field, '/nowhere{Enter}');
    expect(browse).toHaveBeenLastCalledWith('/nowhere');
    expect(await screen.findByRole('alert')).toHaveTextContent('/nowhere does not exist');
  });

  it('shows Windows paths as the host writes them', async () => {
    const win: Record<string, FolderPickerListing> = {
      'C:\\Users\\me': {
        path: 'C:\\Users\\me', display: '~', parent: 'C:\\Users', sep: '\\',
        crumbs: [{ name: '~', path: 'C:\\Users\\me' }],
        places: [{ name: 'Home', path: 'C:\\Users\\me' }, { name: 'C:', path: 'C:\\' }, { name: 'D:', path: 'D:\\' }],
        folders: [folder('C:\\Users\\me\\code', '~\\code')],
      },
      'C:\\Users\\me\\code': {
        path: 'C:\\Users\\me\\code', display: '~\\code', parent: 'C:\\Users\\me', sep: '\\',
        crumbs: [{ name: '~', path: 'C:\\Users\\me' }, { name: 'code', path: 'C:\\Users\\me\\code' }],
        places: [], folders: [],
      },
    };
    const sel = vi.fn();
    render(<Harness host={win} alias={{ '~/': 'C:\\Users\\me', '~\\': 'C:\\Users\\me' }} start="C:\Users\me" onSelection={sel} />);
    const field = screen.getByRole('textbox', { name: 'Folder path' });
    expect(field).toHaveValue('~\\');
    expect(within(screen.getByRole('group', { name: 'Places' })).getByRole('button', { name: 'D:' })).toBeInTheDocument();
    await userEvent.type(field, 'co{Tab}');
    expect(field).toHaveValue('~\\code\\');
    await userEvent.clear(field);
    await userEvent.type(field, '~/code');
    await waitFor(() => expect(screen.queryByRole('alert')).not.toBeInTheDocument());
    await waitFor(() => expect(sel).toHaveBeenLastCalledWith({ path: 'C:\\Users\\me\\code', display: '~\\code', name: 'code' }));
  });

  it('makes a new folder and opens it', async () => {
    const host = structuredClone(mac);
    const create = vi.fn(async (name: string) => {
      const f = folder(`/Users/me/work/${name}`, `~/work/${name}`);
      host['/Users/me/work'].folders.push(f);
      host[f.path] = { ...host['/Users/me/work'], path: f.path, display: f.display, folders: [] };
      return f;
    });
    const browse = vi.fn();
    render(<Harness host={host} start="/Users/me/work" onCreate={create} onBrowse={browse} />);
    await userEvent.click(screen.getByRole('button', { name: 'New folder' }));
    await userEvent.type(screen.getByRole('textbox', { name: 'New folder name' }), 'fresh{Enter}');
    expect(create).toHaveBeenCalledWith('fresh');
    await waitFor(() => expect(browse).toHaveBeenLastCalledWith('/Users/me/work/fresh'));
    await waitFor(() => expect(screen.getByRole('textbox', { name: 'Folder path' })).toHaveValue('~/work/fresh/'));
  });

  it('shows why a new folder failed', async () => {
    const create = vi.fn(async () => { throw new Error('~/work/app already exists'); });
    render(<Harness start="/Users/me/work" onCreate={create} />);
    await userEvent.click(screen.getByRole('button', { name: 'New folder' }));
    await userEvent.type(screen.getByRole('textbox', { name: 'New folder name' }), 'app');
    await act(() => userEvent.click(screen.getByRole('button', { name: 'Create' })));
    expect(await screen.findByRole('alert')).toHaveTextContent('already exists');
  });
});
