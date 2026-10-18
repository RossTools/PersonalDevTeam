import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { Project } from '../api/gen/Project';
import { renderWithQuery, Reply, stubApi } from '../test-utils';
import { FileViewer, FolderTree, useFolderView } from './FolderView';

vi.mock('./CodeViewer', () => ({ CodeViewer: ({ name, text }: { name: string; text: string }) => <pre data-testid="code" data-name={name}>{text}</pre> }));
afterEach(() => vi.unstubAllGlobals());

const project: Project = { id: 'p', name: 'PDT', repoPath: '/r', baseBranch: 'main', createdAt: 1, key: 'PDT', githubRepo: null };
const entry = (path: string, dir = false, status: string | null = null) => ({ name: path.split('/').pop()!, path, dir, status });

function Harness() {
  const state = useFolderView([project], null);
  return <><FolderTree state={state} /><main><FileViewer state={state} /></main></>;
}

describe('folder view', () => {
  it('browses folders lazily and shows a file read-only', async () => {
    stubApi({
      'GET /api/projects/p/worktrees': [],
      'GET /api/projects/p/files?path=': [entry('src', true, 'M'), entry('README.md')],
      'GET /api/projects/p/files?path=src': [entry('src/main.rs', false, '??')],
      'GET /api/projects/p/files/content?path=src%2Fmain.rs': { path: 'src/main.rs', size: 13, status: '??', text: 'fn main() {}\n', binary: false, tooLarge: false },
      'GET /api/projects/p/files/content?path=README.md': { path: 'README.md', size: 4, status: null, text: null, binary: true, tooLarge: false },
    });
    renderWithQuery(<Harness />);
    expect(screen.getByText(/Pick a file/)).toBeInTheDocument();
    expect(await screen.findByRole('button', { name: 'Place: PDT › Repository' })).toBeInTheDocument();
    await userEvent.click(await screen.findByRole('row', { name: /src/ }));
    expect(screen.getByRole('row', { name: /src/ })).toHaveAttribute('aria-expanded', 'true');
    await userEvent.click(await screen.findByRole('row', { name: /main\.rs/ }));
    await waitFor(() => expect(screen.getByTestId('code')).toHaveTextContent('fn main() {}'));
    expect(screen.getByTestId('code')).toHaveAttribute('data-name', 'src/main.rs');
    expect(screen.getByRole('row', { name: /main\.rs/ })).toHaveAttribute('aria-selected', 'true');
    await userEvent.click(screen.getByRole('row', { name: /README/ }));
    expect(await screen.findByText('This is a binary file.')).toBeInTheDocument();
  });

  it('marks git status, strikes deleted files and shows a folder that failed to load', async () => {
    stubApi({
      'GET /api/projects/p/worktrees': [],
      'GET /api/projects/p/files?path=': [entry('gone.txt', false, 'D'), entry('locked', true)],
      'GET /api/projects/p/files?path=locked': new Reply(500, { code: 'internal', message: 'permission denied' }),
    });
    renderWithQuery(<Harness />);
    const gone = await screen.findByRole('row', { name: /gone\.txt/ });
    expect(gone).toHaveTextContent('D');
    expect(within(gone).getByText('gone.txt')).toHaveClass('line-through');
    await userEvent.click(screen.getByRole('row', { name: /locked/ }));
    expect(await screen.findByRole('alert')).toHaveTextContent('permission denied');
  });

  it('shows an error when the checkout cannot be listed', async () => {
    stubApi({ 'GET /api/projects/p/worktrees': [], 'GET /api/projects/p/files?path=': new Reply(404, { code: 'not_found', message: 'checkout is missing' }) });
    renderWithQuery(<Harness />);
    expect(await screen.findByRole('alert')).toHaveTextContent('checkout is missing');
  });
});
