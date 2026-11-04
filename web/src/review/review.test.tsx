import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import { EditorView } from '@codemirror/view';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { ChangedFile } from '../api/gen/ChangedFile';
import type { Changes } from '../api/gen/Changes';
import type { MergePreview } from '../api/gen/MergePreview';
import type { ReviewComment } from '../api/gen/ReviewComment';
import { renderWithQuery, Reply, stubApi, type Call } from '../test-utils';
import { project, ticket } from '../tickets/fixtures';
import { ChangesView } from './ChangesView';
import { MergeDialog } from './MergeDialog';
import { ChangesButton } from './ChangesButton';
import { OpenChangesContext } from './open';
import { changesKey } from './queries';
import { clearViewStates } from '../common/viewState';

beforeEach(() => localStorage.clear());
afterEach(() => vi.unstubAllGlobals());

const changes = (over: Partial<Changes> = {}): Changes => ({
  worktreeId: 'w1', branch: 'pdt/ALP-1-fix-login', base: 'main', mergeBase: 'abcdef1234', head: 'fff', dirty: false,
  attemptId: 'a1', ticketId: 't1', attemptActive: true, approved: false,
  // In path order, as the API sends them.
  files: [
    { path: 'README.md', oldPath: null, status: 'added', untracked: true, binary: false, added: 2, removed: 0, version: ':c', generated: false, uncommitted: true },
    { path: 'src/login.ts', oldPath: null, status: 'modified', untracked: false, binary: false, added: 3, removed: 1, version: 'a:b', generated: false, uncommitted: false },
    { path: 'src/old.ts', oldPath: null, status: 'deleted', untracked: false, binary: false, added: 0, removed: 4, version: 'd:', generated: false, uncommitted: false },
  ],
  ...over,
});

const comment = (over: Partial<ReviewComment> = {}): ReviewComment => ({
  id: 'c1', worktreeId: 'w1', parentId: null, path: 'src/login.ts', side: 'new', line: 2, endLine: 2, lineText: 'gone line',
  body: 'Rename this', author: 'user', sentAt: null, sessionId: null, resolvedAt: null, createdAt: Date.now(), updatedAt: Date.now(),
  ...over,
});

const findTree = () => screen.findByRole('treegrid', { name: 'Changed files' });

const t1 = ticket(1, { title: 'Fix login', stage: 'in_progress' });

function stub(extra: Parameters<typeof stubApi>[0] = {}, c: Changes = changes(), comments: ReviewComment[] = []): Call[] {
  return stubApi({
    'GET /api/worktrees/w1/changes': c,
    'GET /api/worktrees/w1/comments': comments,
    'GET /api/worktrees/w1/changes/file': (call: Call) => {
      const path = new URL(call.path, 'http://x').searchParams.get('path');
      return { path, old: 'a\nb\n', new: 'a\nB\nc\n', binary: false, tooLarge: false };
    },
    'GET /api/tickets/t1': t1,
    [`GET /api/projects/${project.id}/sessions`]: [],
    ...extra,
  });
}

const region = (path: string) => screen.getByRole('region', { name: path });
const findRegion = (path: string) => screen.findByRole('region', { name: path });
const treeRow = (tree: HTMLElement, name: string) => within(tree).getByText(name).closest('[role="row"]')!;
const fileCalls = (calls: Call[]) => calls.filter((c) => c.path.startsWith('/api/worktrees/w1/changes/file')).map((c) => new URL(c.path, 'http://x').searchParams.get('path'));

/** jsdom lays nothing out: each file's section sits 1000px below the one before. */
function layout(order = ['src/login.ts', 'src/old.ts', 'README.md']) {
  vi.spyOn(HTMLElement.prototype, 'offsetTop', 'get').mockImplementation(function (this: HTMLElement) {
    const p = this.dataset.path;
    return p ? Math.max(0, order.indexOf(p)) * 1000 : 0;
  });
}

/** An IntersectionObserver that says which sections are near the visible page. */
function nearOnly(near: (path: string | null) => boolean) {
  class FakeObserver {
    constructor(private cb: IntersectionObserverCallback) {}
    observe(el: Element) {
      this.cb([{ target: el, isIntersecting: near(el.getAttribute('data-path')) } as IntersectionObserverEntry], this as unknown as IntersectionObserver);
    }
    unobserve() {}
    disconnect() {}
    takeRecords() { return []; }
  }
  vi.stubGlobal('IntersectionObserver', FakeObserver);
}

describe('ChangesView', () => {
  afterEach(() => {
    vi.restoreAllMocks();
    window.history.replaceState(null, '', '/');
  });

  it('shows every changed file on one page, in the file list\'s order (by folder), with the list beside it', async () => {
    stub();
    renderWithQuery(<ChangesView worktreeId="w1" ticket={t1} />);
    const tree = await findTree();
    expect(within(tree).getByText('src/')).toBeInTheDocument();
    expect(within(tree).getByText('login.ts')).toBeInTheDocument();
    expect(screen.getByText(/3 files vs/)).toBeInTheDocument();
    expect(screen.getByText(/0 \/ 3 viewed/)).toBeInTheDocument();
    const sections = await screen.findAllByRole('region', { name: /\.(md|ts)$/ });
    expect(sections.map((s) => s.getAttribute('aria-label'))).toEqual(['src/login.ts', 'src/old.ts', 'README.md']);
    for (const s of sections) expect(within(s).getByRole('checkbox', { name: 'Viewed' })).toBeInTheDocument();
    expect(within(region('README.md')).getByText('untracked')).toBeInTheDocument();
    // Every file's diff is drawn (nothing here says which are near).
    await waitFor(() => expect(screen.getAllByTestId('code-diff')).toHaveLength(3));
    // One layout for the whole page.
    expect(screen.getAllByRole('radiogroup', { name: 'Diff layout' })).toHaveLength(1);
  });

  it('marks a file uncommitted in its header', async () => {
    const c = changes();
    c.files[1] = { ...c.files[1]!, uncommitted: true };
    stub({}, c);
    renderWithQuery(<ChangesView worktreeId="w1" ticket={t1} />);
    expect(await within(await findRegion('src/login.ts')).findByText('Uncommitted')).toBeInTheDocument();
    expect(within(region('src/old.ts')).queryByText('Uncommitted')).not.toBeInTheDocument();
  });

  it('shows a Markdown file rendered, before and after side by side', async () => {
    stub({
      'GET /api/worktrees/w1/changes/file': { path: 'README.md', old: '# Old title\n', new: '# New title\n', binary: false, tooLarge: false },
    });
    renderWithQuery(<ChangesView worktreeId="w1" ticket={t1} />);
    const readme = await findRegion('README.md');
    await userEvent.click(within(readme).getByRole('radio', { name: 'Rendered' }));
    expect(within(screen.getByRole('region', { name: 'Before' })).getByRole('heading', { name: 'Old title' })).toBeInTheDocument();
    expect(within(screen.getByRole('region', { name: 'After' })).getByRole('heading', { name: 'New title' })).toBeInTheDocument();
    expect(within(readme).queryByRole('button', { name: 'Whole file' })).not.toBeInTheDocument();
    // Other files keep their diff and their own options.
    expect(within(region('src/login.ts')).getByRole('button', { name: 'Whole file' })).toBeInTheDocument();
    expect(within(region('src/login.ts')).queryByRole('radio', { name: 'Rendered' })).not.toBeInTheDocument();
    await userEvent.click(within(readme).getByRole('radio', { name: 'Diff' }));
    expect(screen.queryByRole('region', { name: 'After' })).not.toBeInTheDocument();
  });

  it('collapses a file marked viewed, which survives a reload until the file changes', async () => {
    stub();
    const { unmount } = renderWithQuery(<ChangesView worktreeId="w1" ticket={t1} />);
    const login = await findRegion('src/login.ts');
    await waitFor(() => expect(within(login).getByTestId('code-diff')).toBeInTheDocument());
    await userEvent.click(within(login).getByRole('checkbox', { name: 'Viewed' }));
    expect(screen.getByText(/1 \/ 3 viewed/)).toBeInTheDocument();
    expect(within(login).queryByTestId('file-body')).not.toBeInTheDocument();
    expect(within(login).getByRole('button', { name: 'Expand src/login.ts' })).toHaveAttribute('aria-expanded', 'false');
    // The chevron opens it again without unmarking it.
    await userEvent.click(within(login).getByRole('button', { name: 'Expand src/login.ts' }));
    expect(within(login).getByTestId('file-body')).toBeInTheDocument();
    expect(within(login).getByRole('checkbox', { name: 'Viewed' })).toBeChecked();
    unmount();
    // A new browser session: what the page remembered for this one (the chevron) is gone, Viewed stays.
    clearViewStates();

    vi.unstubAllGlobals();
    stub();
    const again = renderWithQuery(<ChangesView worktreeId="w1" ticket={t1} />);
    expect(await screen.findByText(/1 \/ 3 viewed/)).toBeInTheDocument();
    expect(within(await findRegion('src/login.ts')).queryByTestId('file-body')).not.toBeInTheDocument();
    // Unticking opens it.
    await userEvent.click(within(region('src/login.ts')).getByRole('checkbox', { name: 'Viewed' }));
    expect(within(region('src/login.ts')).getByTestId('file-body')).toBeInTheDocument();
    await userEvent.click(within(region('src/login.ts')).getByRole('checkbox', { name: 'Viewed' }));
    again.unmount();

    vi.unstubAllGlobals();
    const c = changes();
    c.files[1] = { ...c.files[1]!, version: 'a:changed' };
    stub({}, c);
    renderWithQuery(<ChangesView worktreeId="w1" ticket={t1} />);
    expect(await screen.findByText(/0 \/ 3 viewed/)).toBeInTheDocument();
    expect(within(await findRegion('src/login.ts')).getByTestId('file-body')).toBeInTheDocument();
  });

  it('scrolls to a file picked in the list, follows the scroll, and puts the file in the address', async () => {
    layout();
    stub();
    renderWithQuery(<ChangesView worktreeId="w1" ticket={t1} />);
    const tree = await findTree();
    const page = screen.getByTestId('changes-page');
    await waitFor(() => expect(treeRow(tree, 'login.ts')).toHaveAttribute('aria-selected', 'true'));
    await userEvent.click(within(tree).getByText('README.md'));
    expect(page.scrollTop).toBe(2000);
    expect(treeRow(tree, 'README.md')).toHaveAttribute('aria-selected', 'true');
    expect(new URLSearchParams(window.location.search).get('file')).toBe('README.md');
    // Scroll-spy: scrolling back up highlights the file in view.
    page.scrollTop = 1200;
    fireEvent.scroll(page);
    await waitFor(() => expect(treeRow(tree, 'old.ts')).toHaveAttribute('aria-selected', 'true'));
    expect(treeRow(tree, 'README.md')).not.toHaveAttribute('aria-selected', 'true');
  });

  it('moves between files with ] and [, and v marks the file in view viewed and moves on', async () => {
    layout();
    stub();
    renderWithQuery(<ChangesView worktreeId="w1" ticket={t1} />);
    const tree = await findTree();
    const page = screen.getByTestId('changes-page');
    page.focus();
    await userEvent.keyboard(']');
    expect(page.scrollTop).toBe(1000);
    expect(treeRow(tree, 'old.ts')).toHaveAttribute('aria-selected', 'true');
    await userEvent.keyboard(']');
    expect(page.scrollTop).toBe(2000);
    await userEvent.keyboard(']'); // the last file: stays
    expect(page.scrollTop).toBe(2000);
    await userEvent.keyboard('[[');
    await userEvent.keyboard('[[');
    expect(page.scrollTop).toBe(0);
    await userEvent.keyboard('v');
    expect(within(region('src/login.ts')).getByRole('checkbox', { name: 'Viewed' })).toBeChecked();
    expect(within(region('src/login.ts')).queryByTestId('file-body')).not.toBeInTheDocument();
    expect(page.scrollTop).toBe(1000);
    expect(treeRow(tree, 'old.ts')).toHaveAttribute('aria-selected', 'true');
  });

  it('opens a link to a file at that file', async () => {
    layout();
    window.history.replaceState(null, '', '/sessions/ws/tab?file=src%2Fold.ts&line=2');
    stub();
    renderWithQuery(<ChangesView worktreeId="w1" ticket={t1} />);
    const tree = await findTree();
    await waitFor(() => expect(screen.getByTestId('changes-page').scrollTop).toBe(1000));
    expect(treeRow(tree, 'old.ts')).toHaveAttribute('aria-selected', 'true');
  });

  it('can hide the file list, keeping the page where it was', async () => {
    layout();
    stub();
    renderWithQuery(<ChangesView worktreeId="w1" ticket={t1} />);
    const tree = await findTree();
    const page = screen.getByTestId('changes-page');
    await userEvent.click(within(tree).getByText('old.ts'));
    expect(page.scrollTop).toBe(1000);
    const old = region('src/old.ts');
    await userEvent.click(within(old).getByRole('button', { name: 'Whole file' }));

    await userEvent.click(screen.getByRole('button', { name: 'Hide the file list' }));
    expect(screen.queryByRole('treegrid', { name: 'Changed files' })).not.toBeInTheDocument();
    expect(screen.queryByRole('separator', { name: 'Resize the file list' })).not.toBeInTheDocument();
    // The same page, not a new one: its scroll, its sections and their choices stay.
    expect(screen.getByTestId('changes-page')).toBe(page);
    expect(page.scrollTop).toBe(1000);
    expect(region('src/old.ts')).toBe(old);
    expect(within(old).getByRole('button', { name: 'Changes only' })).toBeInTheDocument();

    await userEvent.click(screen.getByRole('button', { name: 'Show the file list' }));
    expect(screen.getByTestId('changes-page')).toBe(page);
    expect(page.scrollTop).toBe(1000);
    expect(treeRow(await findTree(), 'old.ts')).toHaveAttribute('aria-selected', 'true');
  });

  it('draws only the diffs near the visible page', async () => {
    nearOnly((path) => path === 'README.md');
    const calls = stub();
    renderWithQuery(<ChangesView worktreeId="w1" ticket={t1} />);
    expect(await within(await findRegion('README.md')).findByTestId('code-diff')).toBeInTheDocument();
    expect(within(region('src/login.ts')).queryByTestId('code-diff')).not.toBeInTheDocument();
    expect(within(region('src/old.ts')).queryByTestId('code-diff')).not.toBeInTheDocument();
    // The others hold their place, and their sides aren't fetched yet.
    expect(within(region('src/login.ts')).getByTestId('file-body')).toBeInTheDocument();
    expect(fileCalls(calls)).toEqual(['README.md']);
  });

  it('starts generated, lock, binary and very large files collapsed behind Load diff', async () => {
    const file = (path: string, over: Partial<ChangedFile> = {}): ChangedFile => ({
      path, oldPath: null, status: 'modified', untracked: false, binary: false, added: 2, removed: 1, version: path, generated: false, uncommitted: false, ...over,
    });
    const calls = stub({}, changes({
      files: [
        file('gen/api.ts', { generated: true }),
        file('img.png', { binary: true }),
        file('pnpm-lock.yaml', { generated: true, added: 300 }),
        file('src/big.ts', { added: 4000 }),
        file('src/small.ts'),
      ],
    }));
    renderWithQuery(<ChangesView worktreeId="w1" ticket={t1} />);
    const lock = await findRegion('pnpm-lock.yaml');
    expect(within(lock).getByText('A generated file; its diff is hidden.')).toBeInTheDocument();
    expect(within(lock).getByText('generated')).toBeInTheDocument();
    expect(within(region('gen/api.ts')).getByRole('button', { name: 'Load diff' })).toBeInTheDocument();
    expect(within(region('src/big.ts')).getByText('A large diff; it is hidden.')).toBeInTheDocument();
    expect(within(region('img.png')).getByText('A binary file; there is no text diff.')).toBeInTheDocument();
    expect(within(region('img.png')).queryByRole('button', { name: 'Load diff' })).not.toBeInTheDocument();
    expect(await within(region('src/small.ts')).findByTestId('code-diff')).toBeInTheDocument();
    expect(fileCalls(calls)).toEqual(['src/small.ts']);
    await userEvent.click(within(lock).getByRole('button', { name: 'Load diff' }));
    expect(await within(lock).findByTestId('code-diff')).toBeInTheDocument();
  });

  it('counts drafts, lists a thread whose lines are gone as outdated in its file, and sends the feedback', async () => {
    const calls = stub({ 'POST /api/worktrees/w1/feedback': { sessionId: 's1', sent: 1 } }, changes(), [comment()]);
    renderWithQuery(<ChangesView worktreeId="w1" ticket={t1} />);
    expect(await screen.findByText('1 draft comment')).toBeInTheDocument();
    const login = await findRegion('src/login.ts');
    expect(await within(login).findByText('The lines this was on have changed.')).toBeInTheDocument();
    expect(within(login).getByText('Rename this')).toBeInTheDocument();
    expect(within(region('README.md')).queryByText('Rename this')).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: /Send feedback/ }));
    await waitFor(() => expect(calls.some((c) => c.method === 'POST' && c.path === '/api/worktrees/w1/feedback')).toBe(true));
    expect(await screen.findByText('Sent 1 comment to the agent.')).toBeInTheDocument();
  });

  it('anchors a thread under its line in its own file, and a new comment goes to that file', async () => {
    const calls = stub({ 'POST /api/worktrees/w1/comments': comment({ id: 'c2', line: 3, endLine: 3, body: 'Why c?' }) }, changes(),
      [comment({ line: 2, endLine: 2, lineText: 'B', body: 'Rename this' })]);
    renderWithQuery(<ChangesView worktreeId="w1" ticket={t1} />);
    const login = await findRegion('src/login.ts');
    const thread = await within(login).findByText('Rename this');
    expect(thread.closest('.cm-review-anchor')).not.toBeNull();
    expect(within(login).queryByText('The lines this was on have changed.')).not.toBeInTheDocument();
    const editor = await waitFor(() => {
      const el = login.querySelectorAll('.cm-editor');
      if (el.length === 0) throw new Error('no editor yet');
      return el[el.length - 1] as HTMLElement;
    });
    const view = EditorView.findFromDOM(editor)!;
    const y = (n: number) => view.documentTop + view.lineBlockAt(view.state.doc.line(n).from).top + 1;
    fireEvent.mouseDown(editor.querySelector('.cm-comment-gutter')!, { clientY: y(3) });
    const box = await within(login).findByRole('textbox', { name: 'Comment on line 3' });
    await userEvent.type(box, 'Why c?');
    await userEvent.click(within(login).getByRole('button', { name: 'Comment' }));
    await waitFor(() => expect(calls.some((c) => c.method === 'POST' && c.path === '/api/worktrees/w1/comments')).toBe(true));
    expect(calls.find((c) => c.method === 'POST' && c.path === '/api/worktrees/w1/comments')?.body)
      .toMatchObject({ path: 'src/login.ts', side: 'new', line: 3, endLine: 3, lineText: 'c', body: 'Why c?' });
  });

  it('names who wrote each comment: the user as You, an agent by its name', async () => {
    stub({}, changes(), [comment(), comment({ id: 'c2', parentId: 'c1', body: 'Agreed, rename it', author: 'Reviewer' })]);
    renderWithQuery(<ChangesView worktreeId="w1" ticket={t1} />);
    expect(await screen.findByText('Agreed, rename it')).toBeInTheDocument();
    expect(screen.getByText('You')).toBeInTheDocument();
    expect(screen.getByText('Reviewer')).toBeInTheDocument();
  });

  it('says why feedback could not be sent', async () => {
    stub({ 'POST /api/worktrees/w1/feedback': new Reply(409, { code: 'no_agent_session', message: 'no agent session works on this branch; start one first' }) }, changes(), [comment()]);
    renderWithQuery(<ChangesView worktreeId="w1" ticket={t1} />);
    await userEvent.click(await screen.findByRole('button', { name: /Send feedback/ }));
    expect(await screen.findByText(/no agent session works on this branch/)).toBeInTheDocument();
  });

  it('approves, and shows Approved once the head is approved', async () => {
    const calls = stub({ 'POST /api/attempts/a1/approve': t1 });
    const { unmount } = renderWithQuery(<ChangesView worktreeId="w1" ticket={t1} />);
    await userEvent.click(await screen.findByRole('button', { name: /Approve/ }));
    await waitFor(() => expect(calls.some((c) => c.method === 'POST' && c.path === '/api/attempts/a1/approve')).toBe(true));
    unmount();
    vi.unstubAllGlobals();
    stub({}, changes({ approved: true }));
    renderWithQuery(<ChangesView worktreeId="w1" ticket={t1} />);
    expect(await screen.findByText('Approved')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /^Approve/ })).not.toBeInTheDocument();
  });

  it('keeps the file in view when another one comes before it', async () => {
    stub();
    const { qc } = renderWithQuery(<ChangesView worktreeId="w1" ticket={t1} />);
    const tree = await findTree();
    await waitFor(() => expect(treeRow(tree, 'login.ts')).toHaveAttribute('aria-selected', 'true'));
    const c = changes();
    act(() => qc.setQueryData(changesKey('w1'), {
      ...c, files: [{ path: 'src/auth.ts', oldPath: null, status: 'added', untracked: true, binary: false, added: 1, removed: 0, version: ':a', generated: false, uncommitted: true }, ...c.files],
    }));
    await within(tree).findByText('auth.ts');
    expect(screen.getAllByRole('region', { name: /\.(md|ts)$/ })[0]).toHaveAttribute('aria-label', 'src/auth.ts');
    expect(treeRow(tree, 'login.ts')).toHaveAttribute('aria-selected', 'true');
    expect(treeRow(tree, 'auth.ts')).not.toHaveAttribute('aria-selected', 'true');
  });

  it('offers no Approve or Merge for a worktree without an active attempt', async () => {
    stub({}, changes({ attemptId: null, ticketId: null, attemptActive: false }));
    renderWithQuery(<ChangesView worktreeId="w1" />);
    expect(await screen.findByRole('button', { name: /Send feedback/ })).toBeDisabled();
    expect(screen.queryByRole('button', { name: /Merge/ })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /Approve/ })).not.toBeInTheDocument();
  });

  describe('left and opened again', () => {
    /** Each section is `height` tall (jsdom lays nothing out). */
    const heights = (height: number) =>
      vi.spyOn(HTMLElement.prototype, 'offsetHeight', 'get').mockImplementation(function (this: HTMLElement) {
        return this.dataset.path ? height : 0;
      });
    const scrollTo = async (top: number) => {
      const page = screen.getByTestId('changes-page');
      page.scrollTop = top;
      fireEvent.scroll(page);
      // The scroll-spy reads it on the next frame.
      await act(() => new Promise((r) => requestAnimationFrame(() => r(null))));
    };

    it('shows the same file, Rendered, at the same scroll', async () => {
      layout();
      heights(1000);
      stub();
      const first = renderWithQuery(<ChangesView worktreeId="w1" ticket={t1} />);
      await userEvent.click(within(await findRegion('README.md')).getByRole('radio', { name: 'Rendered' }));
      await userEvent.click(within(region('src/old.ts')).getByRole('button', { name: 'Whole file' }));
      await userEvent.click(within(region('src/login.ts')).getByRole('button', { name: 'Collapse src/login.ts' }));
      await scrollTo(2300);
      first.unmount();

      vi.unstubAllGlobals();
      stub();
      renderWithQuery(<ChangesView worktreeId="w1" ticket={t1} />);
      const tree = await findTree();
      await waitFor(() => expect(screen.getByTestId('changes-page').scrollTop).toBe(2300));
      expect(treeRow(tree, 'README.md')).toHaveAttribute('aria-selected', 'true');
      expect(within(region('README.md')).getByRole('radio', { name: 'Rendered' })).toBeChecked();
      expect(within(region('src/old.ts')).getByRole('button', { name: 'Changes only' })).toBeInTheDocument();
      expect(within(region('src/login.ts')).queryByTestId('file-body')).not.toBeInTheDocument();
    });

    it('falls back to the top of the file when the place in it has gone (new commits since)', async () => {
      layout();
      const tall = heights(1000);
      stub();
      const first = renderWithQuery(<ChangesView worktreeId="w1" ticket={t1} />);
      await userEvent.click(within(await findRegion('README.md')).getByRole('radio', { name: 'Rendered' }));
      await scrollTo(2600);
      first.unmount();

      // The file is shorter now.
      tall.mockRestore();
      heights(300);
      vi.unstubAllGlobals();
      const c = changes();
      c.files[0] = { ...c.files[0]!, version: ':shorter' };
      stub({}, c);
      renderWithQuery(<ChangesView worktreeId="w1" ticket={t1} />);
      const tree = await findTree();
      await waitFor(() => expect(treeRow(tree, 'README.md')).toHaveAttribute('aria-selected', 'true'));
      await waitFor(() => expect(screen.getByTestId('changes-page').scrollTop).toBe(2000));
      expect(within(region('README.md')).getByRole('radio', { name: 'Rendered' })).toBeChecked();
    });

    it('keeps a comment being written, with its text', async () => {
      stub();
      const first = renderWithQuery(<ChangesView worktreeId="w1" ticket={t1} />);
      const login = await findRegion('src/login.ts');
      const editor = await waitFor(() => {
        const el = login.querySelectorAll('.cm-editor');
        if (el.length === 0) throw new Error('no editor yet');
        return el[el.length - 1] as HTMLElement;
      });
      const view = EditorView.findFromDOM(editor)!;
      const y = (n: number) => view.documentTop + view.lineBlockAt(view.state.doc.line(n).from).top + 1;
      fireEvent.mouseDown(editor.querySelector('.cm-comment-gutter')!, { clientY: y(3) });
      await userEvent.type(await within(login).findByRole('textbox', { name: 'Comment on line 3' }), 'Half a thought');
      first.unmount();

      vi.unstubAllGlobals();
      stub();
      renderWithQuery(<ChangesView worktreeId="w1" ticket={t1} />);
      expect(await within(await findRegion('src/login.ts')).findByRole('textbox', { name: 'Comment on line 3' })).toHaveValue('Half a thought');
    });

    it('follows a new link to a file rather than the place it was left at', async () => {
      layout();
      stub();
      const first = renderWithQuery(<ChangesView worktreeId="w1" ticket={t1} />);
      await findTree();
      await scrollTo(2000);
      first.unmount();

      window.history.replaceState(null, '', '/sessions/ws/tab?file=src%2Fold.ts');
      vi.unstubAllGlobals();
      stub();
      renderWithQuery(<ChangesView worktreeId="w1" ticket={t1} />);
      await findTree();
      await waitFor(() => expect(screen.getByTestId('changes-page').scrollTop).toBe(1000));
    });
  });
});

describe('ChangesView on a phone', () => {
  const asPhone = () =>
    vi.stubGlobal('matchMedia', (q: string) => ({ matches: q.includes('max-width'), media: q, addEventListener: () => {}, removeEventListener: () => {} }));
  afterEach(() => {
    vi.restoreAllMocks();
    window.history.replaceState(null, '', '/');
  });

  it('shows the same single page, with Jump to file instead of the list', async () => {
    asPhone();
    layout();
    stub();
    renderWithQuery(<ChangesView worktreeId="w1" ticket={t1} />);
    const sections = await screen.findAllByRole('region', { name: /\.(md|ts)$/ });
    expect(sections).toHaveLength(3);
    expect(screen.queryByRole('treegrid', { name: 'Changed files' })).not.toBeInTheDocument();
    expect(screen.queryByRole('navigation', { name: 'Files' })).not.toBeInTheDocument();
    // No split layout on a phone.
    expect(screen.queryByRole('radio', { name: 'Split' })).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: /Jump to file/ }));
    await userEvent.click(await screen.findByRole('menuitem', { name: 'README.md' }));
    expect(screen.getByTestId('changes-page').scrollTop).toBe(2000);
  });

  it('shows threads inline and writes a new comment in a bottom sheet', async () => {
    asPhone();
    const calls = stub({ 'POST /api/worktrees/w1/comments': comment({ id: 'c2', line: 3, endLine: 3, body: 'Why c?' }) }, changes(),
      [comment({ line: 2, endLine: 2, lineText: 'B', body: 'Rename this' })]);
    renderWithQuery(<ChangesView worktreeId="w1" ticket={t1} />);
    const login = await findRegion('src/login.ts');
    expect(await within(login).findByText('Rename this')).toBeInTheDocument();
    expect(screen.queryByText('The lines this was on have changed.')).not.toBeInTheDocument();

    const editor = await waitFor(() => {
      const el = login.querySelector('.cm-editor');
      if (!el) throw new Error('no editor yet');
      return el as HTMLElement;
    });
    const view = EditorView.findFromDOM(editor)!;
    const y = (n: number) => view.documentTop + view.lineBlockAt(view.state.doc.line(n).from).top + 1;
    fireEvent.mouseDown(editor.querySelector('.cm-comment-gutter')!, { clientY: y(3) });
    const sheet = await screen.findByRole('dialog', { name: 'Comment on line 3' });
    await userEvent.type(within(sheet).getByRole('textbox'), 'Why c?');
    await userEvent.click(within(sheet).getByRole('button', { name: 'Comment' }));
    await waitFor(() => expect(calls.some((c) => c.method === 'POST' && c.path === '/api/worktrees/w1/comments')).toBe(true));
    expect(calls.find((c) => c.method === 'POST' && c.path === '/api/worktrees/w1/comments')?.body)
      .toMatchObject({ path: 'src/login.ts', side: 'new', line: 3, endLine: 3, lineText: 'c', body: 'Why c?' });
    await waitFor(() => expect(screen.queryByRole('dialog', { name: 'Comment on line 3' })).not.toBeInTheDocument());
  });

  it('keeps a reply that failed to save, with the error', async () => {
    stub({ 'POST /api/worktrees/w1/comments': new Reply(500, { code: 'internal', message: 'database is locked' }) }, changes(),
      [comment({ line: 2, endLine: 2, lineText: 'B', body: 'Rename this' })]);
    renderWithQuery(<ChangesView worktreeId="w1" ticket={t1} />);
    const login = await findRegion('src/login.ts');
    expect(await within(login).findByText('Rename this')).toBeInTheDocument();
    await userEvent.click(within(login).getByRole('button', { name: 'Reply' }));
    const box = within(login).getByRole('textbox', { name: 'Reply' });
    await userEvent.type(box, 'Use `login()`');
    await userEvent.click(within(login).getByRole('button', { name: 'Reply' }));
    expect(await screen.findByText('database is locked')).toBeInTheDocument();
    expect(box).toHaveValue('Use `login()`');
  });
});

const preview = (over: Partial<MergePreview> = {}): MergePreview => ({
  method: 'local', branch: 'pdt/ALP-1-fix-login', base: 'main', commits: 2, problem: null, conflicts: [], baseCheckout: null, pr: null,
  ...over,
});

describe('MergeDialog', () => {
  const open = (p: MergePreview, extra: Parameters<typeof stubApi>[0] = {}) => {
    const onClose = vi.fn();
    const calls = stubApi({ 'GET /api/attempts/a1/merge': p, ...extra });
    renderWithQuery(<MergeDialog ticket={t1} attemptId="a1" onClose={onClose} />);
    return { calls, onClose };
  };

  it('shows the commits and the cleanup choices for a clean local merge', async () => {
    open(preview());
    expect(await screen.findByText(/\(2 commits\) into/)).toHaveTextContent(/with a merge commit/);
    expect(screen.getByRole('checkbox', { name: 'Remove the worktree' })).toBeChecked();
    const del = screen.getByRole('checkbox', { name: 'Delete the branch' });
    expect(del).not.toBeChecked();
    await userEvent.click(screen.getByRole('checkbox', { name: 'Remove the worktree' }));
    expect(del).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Merge' })).toBeEnabled();
  });

  it('shows an alert instead of the button for uncommitted changes', async () => {
    open(preview({ problem: 'dirty' }));
    expect(await screen.findByText('Uncommitted changes')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Merge' })).not.toBeInTheDocument();
  });

  it('lists conflicts and sends them to the agent', async () => {
    const { calls, onClose } = open(preview({ problem: 'conflict', conflicts: ['a.txt', 'b.rs'] }), {
      'POST /api/attempts/a1/ask-agent': { sessionId: 's1', sent: 0 },
    });
    expect(await screen.findByText('a.txt')).toBeInTheDocument();
    expect(screen.getByText('b.rs')).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Send to agent' }));
    await waitFor(() => expect(onClose).toHaveBeenCalled());
    expect(calls.some((c) => c.method === 'POST' && c.path === '/api/attempts/a1/ask-agent')).toBe(true);
  });

  it('asks the agent for a pull request when the project merges through them', async () => {
    const { calls, onClose } = open(preview({ method: 'forge', problem: 'no_pr' }), {
      'POST /api/attempts/a1/ask-agent': { sessionId: 's1', sent: 0 },
    });
    expect(await screen.findByText('No pull request yet')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Merge' })).not.toBeInTheDocument();
    expect(screen.queryByRole('checkbox')).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Ask the agent to open one' }));
    await waitFor(() => expect(onClose).toHaveBeenCalled());
    expect(calls.some((c) => c.method === 'POST' && c.path === '/api/attempts/a1/ask-agent')).toBe(true);
    expect(await screen.findByText('Asked the agent to open a pull request.')).toBeInTheDocument();
  });

  it('turns on auto-merge for an open pull request', async () => {
    const { calls, onClose } = open(preview({ method: 'forge', pr: { number: 12, url: 'https://github.com/o/r/pull/12', forge: 'GitHub', queue: true, queueName: 'merge queue', local: false, autoMerge: true } }), {
      'POST /api/attempts/a1/merge': { outcome: 'queued', commit: null },
    });
    expect(await screen.findByText(/through the merge queue/)).toBeInTheDocument();
    // Finishing once it merges is on by default; deleting the branch is not, and goes with it.
    const finish = screen.getByRole('checkbox', { name: 'Move to Done and clean up once it merges' });
    expect(finish).toBeChecked();
    const del = screen.getByRole('checkbox', { name: 'Delete the branch' });
    expect(del).not.toBeChecked();
    await userEvent.click(del);
    await userEvent.click(screen.getByRole('button', { name: 'Add to merge queue' }));
    await waitFor(() => expect(onClose).toHaveBeenCalled());
    expect(calls.find((c) => c.method === 'POST' && c.path === '/api/attempts/a1/merge')?.body).toEqual({ finishOnMerge: true, deleteBranch: true, by: 'user' });
    expect(await screen.findByText(/then ALP-1 moves to Done/)).toBeInTheDocument();
  });

  it('names the forge and what it calls its merge queue', async () => {
    const pr = { number: 7, url: 'https://git.example.org/o/r/-/merge_requests/7', forge: 'GitLab', queue: true, queueName: 'merge train', local: false, autoMerge: true };
    open(preview({ method: 'forge', pr }), {});
    expect(await screen.findByText(/merges into/)).toHaveTextContent(/on GitLab through the merge train/);
    expect(screen.getByRole('button', { name: 'Add to merge train' })).toBeInTheDocument();
  });

  it('says which forge can’t merge a pull request', async () => {
    const pr = { number: 7, url: 'u', forge: 'GitLab', queue: false, queueName: 'merge train', local: false, autoMerge: true };
    open(preview({ method: 'forge', problem: 'pr_not_mergeable', pr }), {});
    expect(await screen.findByText(/GitLab can’t merge pull request #7/)).toBeInTheDocument();
  });

  it('sets a local change request to merge once its CI passes (PDT-178)', async () => {
    const { onClose } = open(preview({ method: 'forge', pr: { number: 3, url: '', forge: 'PDT', queue: false, queueName: 'merge queue', local: true, autoMerge: true } }), {
      'POST /api/attempts/a1/merge': { outcome: 'queued', commit: null },
    });
    expect(await screen.findByText(/merges into/)).toHaveTextContent(/here \(a fast-forward/);
    expect(screen.getByRole('dialog', { name: 'Merge change request #3' })).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Merge when CI passes' }));
    await waitFor(() => expect(onClose).toHaveBeenCalled());
    expect(await screen.findByText(/Change request #3 merges into main once its CI passes/)).toBeInTheDocument();
  });

  it('merges a pull request when CI passes where the repository has no auto-merge (PDT-297)', async () => {
    const pr = { number: 12, url: 'https://github.com/o/r/pull/12', forge: 'GitHub', queue: false, queueName: 'merge queue', local: false, autoMerge: false };
    const { onClose } = open(preview({ method: 'forge', pr }), { 'POST /api/attempts/a1/merge': { outcome: 'queued', commit: null } });
    expect(await screen.findByText(/doesn’t allow auto-merge, so PDT merges it now if its CI passed, or once it passes/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Merge when CI passes' }));
    await waitFor(() => expect(onClose).toHaveBeenCalled());
    expect(await screen.findByText(/PR #12 is set to merge. PDT merges it once its CI passes/)).toBeInTheDocument();
  });

  it('says PDT holds a pull request already set to merge without auto-merge (PDT-297)', async () => {
    const pr = { number: 12, url: 'u', forge: 'GitHub', queue: false, queueName: 'merge queue', local: false, autoMerge: false };
    open(preview({ method: 'forge', problem: 'pr_queued', pr }), {});
    expect(await screen.findByText(/already set to merge; PDT merges it once its CI passes/)).toBeInTheDocument();
  });

  it('turning off finishing once it merges turns off deleting the branch too', async () => {
    const { calls } = open(preview({ method: 'forge', pr: { number: 12, url: 'https://github.com/o/r/pull/12', forge: 'GitHub', queue: false, queueName: 'merge queue', local: false, autoMerge: true } }), {
      'POST /api/attempts/a1/merge': { outcome: 'queued', commit: null },
    });
    await userEvent.click(await screen.findByRole('checkbox', { name: 'Delete the branch' }));
    await userEvent.click(screen.getByRole('checkbox', { name: 'Move to Done and clean up once it merges' }));
    expect(screen.getByRole('checkbox', { name: 'Delete the branch' })).toBeDisabled();
    expect(screen.getByText(/offers to move ALP-1 to Done/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Turn on auto-merge' }));
    await waitFor(() => expect(calls.some((c) => c.method === 'POST' && c.path === '/api/attempts/a1/merge')).toBe(true));
    expect(calls.find((c) => c.method === 'POST' && c.path === '/api/attempts/a1/merge')?.body).toEqual({ finishOnMerge: false, deleteBranch: false, by: 'user' });
  });

  it('merges locally and, keeping the worktree, moves the ticket to Done', async () => {
    const { calls } = open(preview(), {
      'POST /api/attempts/a1/merge': { outcome: 'merged', commit: 'abc' },
      'GET /api/tickets/t1': t1,
      'PATCH /api/tickets/t1': { ...t1, stage: 'done' },
    });
    await userEvent.click(await screen.findByRole('checkbox', { name: 'Remove the worktree' }));
    await userEvent.click(screen.getByRole('button', { name: 'Merge' }));
    await waitFor(() => expect(calls.some((c) => c.method === 'PATCH' && c.path === '/api/tickets/t1')).toBe(true));
    expect(calls.find((c) => c.method === 'PATCH')?.body).toEqual({ stage: 'done' });
    expect(calls.find((c) => c.method === 'POST' && c.path === '/api/attempts/a1/merge')?.body).toEqual({ style: 'merge', by: 'user' });
    expect(await screen.findByText(/Merged pdt\/ALP-1-fix-login into main/)).toBeInTheDocument();
  });

  it('shows a failed merge in the dialog, which stays open', async () => {
    const message = "git doesn't know who you are, so it can't make the commit.";
    const { onClose } = open(preview(), { 'POST /api/attempts/a1/merge': new Reply(409, { code: 'git_identity_missing', message }) });
    await userEvent.click(await screen.findByRole('button', { name: 'Merge' }));
    const alert = await screen.findByRole('alert');
    expect(alert).toHaveTextContent('The merge didn’t go through');
    expect(alert).toHaveTextContent(message);
    expect(onClose).not.toHaveBeenCalled();
  });

  it('squashes when asked, and remembers it for the project', async () => {
    localStorage.removeItem(`pdt.review.mergeStyle.${t1.projectId}`);
    const { calls } = open(preview(), {
      'POST /api/attempts/a1/merge': { outcome: 'merged', commit: 'abc' },
      'GET /api/tickets/t1': t1,
      'PATCH /api/tickets/t1': { ...t1, stage: 'done' },
    });
    await userEvent.click(await screen.findByRole('radio', { name: 'Squash' }));
    expect(screen.getByText(/\(2 commits\) into/)).toHaveTextContent(/as one new commit/);
    await userEvent.click(screen.getByRole('checkbox', { name: 'Remove the worktree' }));
    await userEvent.click(screen.getByRole('button', { name: 'Squash and merge' }));
    await waitFor(() => expect(calls.some((c) => c.method === 'PATCH' && c.path === '/api/tickets/t1')).toBe(true));
    expect(calls.find((c) => c.method === 'POST' && c.path === '/api/attempts/a1/merge')?.body).toEqual({ style: 'squash', by: 'user' });
    expect(await screen.findByText(/Squashed pdt\/ALP-1-fix-login into main/)).toBeInTheDocument();
    expect(localStorage.getItem(`pdt.review.mergeStyle.${t1.projectId}`)).toBe('squash');
    localStorage.removeItem(`pdt.review.mergeStyle.${t1.projectId}`);
  });
});

describe('ChangesButton', () => {
  it('shows the diff size and opens the changes tab', async () => {
    stub();
    const open = vi.fn();
    renderWithQuery(
      <OpenChangesContext.Provider value={open}>
        <ChangesButton worktreeId="w1" projectId="p1" />
      </OpenChangesContext.Provider>,
    );
    const button = screen.getByRole('button', { name: /Changes/ });
    await waitFor(() => expect(button).toHaveTextContent('+5−5'));
    await userEvent.click(button);
    expect(open).toHaveBeenCalledWith({ id: 'w1', projectId: 'p1' }, { here: true });
  });
});
