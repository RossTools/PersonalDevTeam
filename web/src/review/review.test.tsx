import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Changes } from '../api/gen/Changes';
import type { MergePreview } from '../api/gen/MergePreview';
import type { ReviewComment } from '../api/gen/ReviewComment';
import { renderWithQuery, Reply, stubApi, type Call } from '../test-utils';
import { project, ticket } from '../tickets/fixtures';
import { ChangesView } from './ChangesView';
import { MergeDialog } from './MergeDialog';

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

  it('offers no Approve or Merge for a worktree without an active attempt', async () => {
    stub({}, changes({ attemptId: null, ticketId: null, attemptActive: false }));
    renderWithQuery(<ChangesView worktreeId="w1" />);
    expect(await screen.findByRole('button', { name: /Send feedback/ })).toBeDisabled();
    expect(screen.queryByRole('button', { name: /Merge/ })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /Approve/ })).not.toBeInTheDocument();
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
    expect(await screen.findByText(/with a merge commit \(2 commits\)/)).toBeInTheDocument();
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
      'POST /api/attempts/a1/conflicts': { sessionId: 's1', sent: 0 },
    });
    expect(await screen.findByText('a.txt')).toBeInTheDocument();
    expect(screen.getByText('b.rs')).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Send to agent' }));
    await waitFor(() => expect(onClose).toHaveBeenCalled());
    expect(calls.some((c) => c.method === 'POST' && c.path === '/api/attempts/a1/conflicts')).toBe(true);
  });

  it('turns on auto-merge for an open pull request', async () => {
    const { calls, onClose } = open(preview({ method: 'github', pr: { number: 12, url: 'https://github.com/o/r/pull/12', queue: true } }), {
      'POST /api/attempts/a1/merge': { outcome: 'queued', commit: null },
    });
    expect(await screen.findByText(/through the merge queue/)).toBeInTheDocument();
    expect(screen.queryByRole('checkbox')).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Add to merge queue' }));
    await waitFor(() => expect(onClose).toHaveBeenCalled());
    expect(calls.some((c) => c.method === 'POST' && c.path === '/api/attempts/a1/merge')).toBe(true);
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
    expect(await screen.findByText(/Merged pdt\/ALP-1-fix-login into main/)).toBeInTheDocument();
  });
});
