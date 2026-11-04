import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { Project } from '../api/gen/Project';
import type { SearchResult } from '../api/gen/SearchResult';
import { renderWithQuery, Reply, stubApi } from '../test-utils';
import { FileViewer, FolderTree, useFolderView } from './FolderView';
import type { CodeTarget } from './CodeViewer';

vi.mock('./CodeViewer', () => ({
  CodeViewer: ({ name, text, at }: { name: string; text: string; at?: CodeTarget }) =>
    <pre data-testid="code" data-name={name} data-at={at ? `${at.line}:${at.from}-${at.to}` : ''}>{text}</pre>,
}));
afterEach(() => { vi.unstubAllGlobals(); });

const project: Project = { id: 'p', name: 'PDT', repoPath: '/r', baseBranch: 'main', createdAt: 1, key: 'PDT', forge: null, kind: 'repo' };

function Harness() {
  const state = useFolderView([project], null);
  return <><FolderTree state={state} /><main><FileViewer state={state} /></main></>;
}

const found: SearchResult = {
  files: [
    { path: 'src/a.rs', count: 2, lines: [
      { line: 3, text: '    fn needle() {}', offset: 0, cutEnd: false, ranges: [[7, 13]] },
      { line: 9, text: 'needle', offset: 0, cutEnd: false, ranges: [[0, 6]] },
    ] },
    { path: 'web/b.ts', count: 60, lines: [{ line: 1, text: 'x needle y', offset: 100, cutEnd: true, ranges: [[2, 8]] }] },
  ],
  capped: true,
  timedOut: false,
};

const routes = {
  'GET /api/projects/p/worktrees': [],
  'GET /api/projects/p/files?path=': [{ name: 'src', path: 'src', dir: true, status: null }],
  'GET /api/projects/p/files/content?path=src%2Fa.rs': { path: 'src/a.rs', size: 1, status: null, text: 'fn needle() {}\n', binary: false, tooLarge: false, hash: 'h' },
  'GET /api/projects/p/files/content?path=web%2Fb.ts': { path: 'web/b.ts', size: 1, status: null, text: 'x needle y\n', binary: false, tooLarge: false, hash: 'h' },
};

describe('search in files', () => {
  it('groups the matches by file and opens one at its line', async () => {
    const calls = stubApi({ ...routes, 'GET /api/projects/p/files/search': found });
    renderWithQuery(<Harness />);
    expect(await screen.findByRole('row', { name: /src/ })).toBeInTheDocument();
    await userEvent.type(screen.getByRole('searchbox', { name: 'Search in files' }), 'needle');
    expect(await screen.findByText('Showing the first 3 matches, in 2 files.')).toBeInTheDocument();
    expect(screen.queryByRole('row', { name: /src/ })).not.toBeInTheDocument();
    // One request, once typing paused.
    expect(calls.filter((c) => c.path.includes('/files/search')).map((c) => c.path)).toEqual(['/api/projects/p/files/search?q=needle']);

    const group = screen.getByRole('button', { name: 'src/a.rs' });
    expect(group).toHaveAttribute('aria-expanded', 'true');
    expect(screen.getByText('59 more lines in this file')).toBeInTheDocument();
    const marks = document.querySelectorAll('mark');
    expect([...marks].map((m) => m.textContent)).toEqual(['needle', 'needle', 'needle']);

    await userEvent.click(screen.getByRole('button', { name: /^3\s*fn needle/ }));
    await waitFor(() => expect(screen.getByTestId('code')).toHaveAttribute('data-name', 'src/a.rs'));
    expect(screen.getByTestId('code')).toHaveAttribute('data-at', '3:7-13');
    // The start of a cut line counts in where the match is.
    await userEvent.click(screen.getByRole('button', { name: /^1\s*…x needle y…/ }));
    await waitFor(() => expect(screen.getByTestId('code')).toHaveAttribute('data-at', '1:102-108'));

    // A group folds.
    await userEvent.click(group);
    expect(group).toHaveAttribute('aria-expanded', 'false');
    expect(screen.queryByRole('button', { name: /^3\s*fn needle/ })).not.toBeInTheDocument();
  });

  it('sends the options, and shows no results, a bad pattern and a search that ran out of time', async () => {
    const calls = stubApi({
      ...routes,
      'GET /api/projects/p/files/search': (c: { path: string }) => {
        const q = new URLSearchParams(c.path.split('?')[1]);
        if (q.get('q') === 'a(') return new Reply(400, { code: 'bad_pattern', message: "-e option, 'a(': parentheses not balanced" });
        if (q.get('q') === 'slow') return { files: [], capped: false, timedOut: true };
        return { files: [], capped: false, timedOut: false };
      },
    });
    renderWithQuery(<Harness />);
    const box = await screen.findByRole('searchbox', { name: 'Search in files' });
    await userEvent.click(screen.getByRole('button', { name: 'Match case' }));
    await userEvent.click(screen.getByRole('button', { name: 'Whole word' }));
    await userEvent.type(box, 'zzz');
    await userEvent.type(screen.getByRole('textbox', { name: 'Files to search' }), '*.rs');
    expect(await screen.findByText('No results.')).toBeInTheDocument();
    await waitFor(() => expect(calls.at(-1)?.path).toBe('/api/projects/p/files/search?q=zzz&case=true&word=true&glob=*.rs'));

    await userEvent.click(screen.getByRole('button', { name: 'Regular expression' }));
    await userEvent.clear(box);
    await userEvent.type(box, 'a(');
    expect(await screen.findByText(/parentheses not balanced/)).toBeInTheDocument();

    await userEvent.clear(box);
    await userEvent.type(box, 'slow');
    expect(await screen.findByText(/The search took too long/)).toBeInTheDocument();

    // Escape clears the search and the files come back.
    await userEvent.type(box, '{Escape}');
    expect(await screen.findByRole('row', { name: /src/ })).toBeInTheDocument();
    expect(within(document.body).queryByText(/The search took too long/)).not.toBeInTheDocument();
  });
});
