import { screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { Project } from '../api/gen/Project';
import { renderWithQuery, stubApi } from '../test-utils';
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
    await userEvent.click(await screen.findByRole('button', { name: /src/ }));
    await userEvent.click(await screen.findByRole('button', { name: /main\.rs/ }));
    await waitFor(() => expect(screen.getByTestId('code')).toHaveTextContent('fn main() {}'));
    expect(screen.getByTestId('code')).toHaveAttribute('data-name', 'src/main.rs');
    await userEvent.click(screen.getByRole('button', { name: /README/ }));
    expect(await screen.findByText('This is a binary file.')).toBeInTheDocument();
  });
});
