import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import { EditorView } from '@codemirror/view';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
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

beforeEach(() => localStorage.clear());
afterEach(() => vi.unstubAllGlobals());

const changes = (over: Partial<Changes> = {}): Changes => ({
  worktreeId: 'w1', branch: 'pdt/ALP-1-fix-login', base: 'main', mergeBase: 'abcdef1234', head: 'fff', dirty: false,
  attemptId: 'a1', ticketId: 't1', attemptActive: true, approved: false,
  // In path order, as the API sends them.
  files: [
    { path: 'README.md', oldPath: null, status: 'added', untracked: true, binary: false, added: 2, removed: 0, version: ':c' },
    { path: 'src/login.ts', oldPath: null, status: 'modified', untracked: false, binary: false, added: 3, removed: 1, version: 'a:b' },
    { path: 'src/old.ts', oldPath: null, status: 'deleted', untracked: false, binary: false, added: 0, removed: 4, version: 'd:' },
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

describe('ChangesView', () => {
  it('lists the changed files by folder with their counts and opens the first', async () => {
    stub();
    renderWithQuery(<ChangesView worktreeId="w1" ticket={t1} />);
    const tree = await findTree();
    expect(within(tree).getByText('src/')).toBeInTheDocument();
    expect(within(tree).getByText('login.ts')).toBeInTheDocument();
    expect(within(tree).getByText('README.md')).toBeInTheDocument();
    expect(screen.getByText(/3 files vs/)).toBeInTheDocument();
    expect(screen.getByText('0 / 3 viewed')).toBeInTheDocument();
    // The first file is shown.
    expect(await screen.findByRole('checkbox', { name: 'Viewed' })).toBeInTheDocument();
    expect(screen.getByText('untracked')).toBeInTheDocument();
  });

  it('shows a Markdown file rendered, before and after side by side', async () => {
    stub({
      'GET /api/worktrees/w1/changes/file': { path: 'README.md', old: '# Old title\n', new: '# New title\n', binary: false, tooLarge: false },
    });
    renderWithQuery(<ChangesView worktreeId="w1" ticket={t1} />);
    await userEvent.click(await screen.findByRole('radio', { name: 'Rendered' }));
    expect(within(screen.getByRole('region', { name: 'Before' })).getByRole('heading', { name: 'Old title' })).toBeInTheDocument();
    expect(within(screen.getByRole('region', { name: 'After' })).getByRole('heading', { name: 'New title' })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Whole file' })).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole('radio', { name: 'Diff' }));
    expect(screen.queryByRole('region', { name: 'After' })).not.toBeInTheDocument();
    // Not offered for other files.
    await userEvent.click(within(await findTree()).getByText('login.ts'));
    expect(await screen.findByRole('checkbox', { name: 'Viewed' })).toBeInTheDocument();
    expect(screen.queryByRole('radio', { name: 'Rendered' })).not.toBeInTheDocument();
  });

  it('marks a file viewed, which survives a reload until the file changes', async () => {
    stub();
    const { unmount } = renderWithQuery(<ChangesView worktreeId="w1" ticket={t1} />);
    await userEvent.click(await screen.findByRole('checkbox', { name: 'Viewed' }));
    expect(screen.getByText('1 / 3 viewed')).toBeInTheDocument();
    unmount();
    vi.unstubAllGlobals();
    const c = changes();
    c.files[0] = { ...c.files[0]!, version: ':changed' };
    stub({}, c);
    renderWithQuery(<ChangesView worktreeId="w1" ticket={t1} />);
    expect(await screen.findByText('0 / 3 viewed')).toBeInTheDocument();
  });

  it('counts drafts, lists a thread whose lines are gone as outdated, and sends the feedback', async () => {
    const calls = stub({ 'POST /api/worktrees/w1/feedback': { sessionId: 's1', sent: 1 } }, changes(), [comment()]);
    renderWithQuery(<ChangesView worktreeId="w1" ticket={t1} />);
    expect(await screen.findByText('1 draft comment')).toBeInTheDocument();
    await userEvent.click(within(await findTree()).getByText('login.ts'));
    expect(await screen.findByText('The lines this was on have changed.')).toBeInTheDocument();
    expect(screen.getByText('Rename this')).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: /Send feedback/ }));
    await waitFor(() => expect(calls.some((c) => c.method === 'POST' && c.path === '/api/worktrees/w1/feedback')).toBe(true));
    expect(await screen.findByText('Sent 1 comment to the agent.')).toBeInTheDocument();
  });

  it('names who wrote each comment: the user as You, an agent by its name', async () => {
    stub({}, changes(), [comment(), comment({ id: 'c2', parentId: 'c1', body: 'Agreed, rename it', author: 'Reviewer' })]);
    renderWithQuery(<ChangesView worktreeId="w1" ticket={t1} />);
    await userEvent.click(within(await findTree()).getByText('login.ts'));
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

  it('keeps showing the first file when another one comes before it', async () => {
    stub();
    const { qc } = renderWithQuery(<ChangesView worktreeId="w1" ticket={t1} />);
    const tree = await findTree();
    const row = (name: string) => within(tree).getByText(name).closest('[role="row"]')!;
    await waitFor(() => expect(row('README.md')).toHaveAttribute('aria-selected', 'true'));
    const c = changes();
    act(() => qc.setQueryData(changesKey('w1'), {
      ...c, files: [{ path: 'AUTHORS', oldPath: null, status: 'added', untracked: true, binary: false, added: 1, removed: 0, version: ':a' }, ...c.files],
    }));
    await within(tree).findByText('AUTHORS');
    expect(row('README.md')).toHaveAttribute('aria-selected', 'true');
    expect(row('AUTHORS')).not.toHaveAttribute('aria-selected', 'true');
  });

  it('offers no Approve or Merge for a worktree without an active attempt', async () => {
    stub({}, changes({ attemptId: null, ticketId: null, attemptActive: false }));
    renderWithQuery(<ChangesView worktreeId="w1" />);
    expect(await screen.findByRole('button', { name: /Send feedback/ })).toBeDisabled();
    expect(screen.queryByRole('button', { name: /Merge/ })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /Approve/ })).not.toBeInTheDocument();
  });
});

describe('ChangesView on a phone', () => {
  const asPhone = () =>
    vi.stubGlobal('matchMedia', (q: string) => ({ matches: q.includes('max-width'), media: q, addEventListener: () => {}, removeEventListener: () => {} }));
  const fileName = () => screen.getByRole('checkbox', { name: 'Viewed', hidden: true }).closest('header')!.querySelector('.font-mono')!.textContent;
  const swipe = (el: Element, dx: number) => {
    fireEvent.touchStart(el, { touches: [{ clientX: 200, clientY: 300 }] });
    fireEvent.touchEnd(el, { changedTouches: [{ clientX: 200 + dx, clientY: 310 }] });
  };

  it('lists the files as a screen; a file opens its own, swipes and buttons step through, Back returns', async () => {
    asPhone();
    stub();
    renderWithQuery(<ChangesView worktreeId="w1" ticket={t1} />);
    const list = await screen.findByRole('grid', { name: 'Changed files' });
    expect(screen.queryByRole('checkbox', { name: 'Viewed' })).not.toBeInTheDocument();
    expect(within(list).getByRole('row', { name: /login\.ts/ })).toHaveTextContent('src');
    await userEvent.click(within(list).getByRole('row', { name: /login\.ts/ }));
    expect(await screen.findByRole('checkbox', { name: 'Viewed' })).toBeInTheDocument();
    expect(fileName()).toBe('src/login.ts');
    expect(screen.queryByRole('grid', { name: 'Changed files' })).not.toBeInTheDocument();
    expect(screen.getByText('2 of 3')).toBeInTheDocument();
    // No split layout on a phone.
    expect(screen.queryByRole('radio', { name: 'Split' })).not.toBeInTheDocument();

    const nav = screen.getByRole('navigation', { name: 'Files' });
    swipe(nav, -120);
    await waitFor(() => expect(fileName()).toBe('src/old.ts'));
    expect(screen.getByRole('button', { name: 'Next file' })).toBeDisabled();
    swipe(nav, -120); // the last file: stays
    expect(fileName()).toBe('src/old.ts');
    swipe(nav, 30); // too short to count
    expect(fileName()).toBe('src/old.ts');
    swipe(nav, 120);
    await waitFor(() => expect(fileName()).toBe('src/login.ts'));
    await userEvent.click(screen.getByRole('button', { name: 'Previous file' }));
    await waitFor(() => expect(fileName()).toBe('README.md'));
    expect(screen.getByRole('button', { name: 'Previous file' })).toBeDisabled();

    // Back (the browser's, the top bar's ←) walks out to the list.
    window.history.back();
    expect(await screen.findByRole('grid', { name: 'Changed files' })).toBeInTheDocument();
    expect(screen.queryByRole('checkbox', { name: 'Viewed' })).not.toBeInTheDocument();
    // All files does too.
    await userEvent.click(within(screen.getByRole('grid', { name: 'Changed files' })).getByRole('row', { name: /old\.ts/ }));
    await userEvent.click(await screen.findByRole('button', { name: /All files/ }));
    expect(await screen.findByRole('grid', { name: 'Changed files' })).toBeInTheDocument();
  });

  it('shows threads inline and writes a new comment in a bottom sheet', async () => {
    asPhone();
    const calls = stub({ 'POST /api/worktrees/w1/comments': comment({ id: 'c2', line: 3, endLine: 3, body: 'Why c?' }) }, changes(),
      [comment({ line: 2, endLine: 2, lineText: 'B', body: 'Rename this' })]);
    renderWithQuery(<ChangesView worktreeId="w1" ticket={t1} />);
    await userEvent.click(within(await screen.findByRole('grid', { name: 'Changed files' })).getByRole('row', { name: /login\.ts/ }));
    expect(await screen.findByText('Rename this')).toBeInTheDocument();
    expect(screen.queryByText('The lines this was on have changed.')).not.toBeInTheDocument();

    const editor = await waitFor(() => {
      const el = document.querySelector('.cm-editor');
      if (!el) throw new Error('no editor yet');
      return el as HTMLElement;
    });
    const view = EditorView.findFromDOM(editor)!;
    const y = (n: number) => view.documentTop + view.lineBlockAt(view.state.doc.line(n).from).top + 1;
    fireEvent.mouseDown(document.querySelector('.cm-comment-gutter')!, { clientY: y(3) });
    const sheet = await screen.findByRole('dialog', { name: 'Comment on line 3' });
    await userEvent.type(within(sheet).getByRole('textbox'), 'Why c?');
    // A swipe on the open sheet stays there: the file and the draft are kept.
    swipe(within(sheet).getByText('Comment on line 3'), -120);
    swipe(within(sheet).getByText('Comment on line 3'), 120);
    expect(fileName()).toBe('src/login.ts');
    expect(screen.getByRole('dialog', { name: 'Comment on line 3' })).toBeInTheDocument();
    expect(within(sheet).getByRole('textbox')).toHaveValue('Why c?');
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
    await userEvent.click(within(await findTree()).getByText('login.ts'));
    expect(await screen.findByText('Rename this')).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Reply' }));
    const box = screen.getByRole('textbox', { name: 'Reply' });
    await userEvent.type(box, 'Use `login()`');
    await userEvent.click(screen.getByRole('button', { name: 'Reply' }));
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
    const { calls, onClose } = open(preview({ method: 'forge', pr: { number: 12, url: 'https://github.com/o/r/pull/12', forge: 'GitHub', queue: true, queueName: 'merge queue', local: false } }), {
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
    const pr = { number: 7, url: 'https://git.example.org/o/r/-/merge_requests/7', forge: 'GitLab', queue: true, queueName: 'merge train', local: false };
    open(preview({ method: 'forge', pr }), {});
    expect(await screen.findByText(/merges into/)).toHaveTextContent(/on GitLab through the merge train/);
    expect(screen.getByRole('button', { name: 'Add to merge train' })).toBeInTheDocument();
  });

  it('says which forge can’t merge a pull request', async () => {
    const pr = { number: 7, url: 'u', forge: 'GitLab', queue: false, queueName: 'merge train', local: false };
    open(preview({ method: 'forge', problem: 'pr_not_mergeable', pr }), {});
    expect(await screen.findByText(/GitLab can’t merge pull request #7/)).toBeInTheDocument();
  });

  it('sets a local change request to merge once its CI passes (PDT-178)', async () => {
    const { onClose } = open(preview({ method: 'forge', pr: { number: 3, url: '', forge: 'PDT', queue: false, queueName: 'merge queue', local: true } }), {
      'POST /api/attempts/a1/merge': { outcome: 'queued', commit: null },
    });
    expect(await screen.findByText(/merges into/)).toHaveTextContent(/here \(a fast-forward/);
    expect(screen.getByRole('dialog', { name: 'Merge change request #3' })).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Merge when CI passes' }));
    await waitFor(() => expect(onClose).toHaveBeenCalled());
    expect(await screen.findByText(/Change request #3 merges into main once its CI passes/)).toBeInTheDocument();
  });

  it('turning off finishing once it merges turns off deleting the branch too', async () => {
    const { calls } = open(preview({ method: 'forge', pr: { number: 12, url: 'https://github.com/o/r/pull/12', forge: 'GitHub', queue: false, queueName: 'merge queue', local: false } }), {
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
