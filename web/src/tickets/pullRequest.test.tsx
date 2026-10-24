import { render, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { AttemptSummary } from '../api/gen/AttemptSummary';
import type { CiStatus } from '../api/gen/CiStatus';
import type { PullRequestStatus } from '../api/gen/PullRequestStatus';
import { renderWithQuery, stubApi } from '../test-utils';
import { BoardCard } from './Board';
import { event, project, summary, ticket } from './fixtures';
import { eventTitle } from './model';
import { PrDetails, prSummary } from './PullRequest';
import { TicketView } from './TicketView';

beforeEach(() => localStorage.clear());
afterEach(() => vi.unstubAllGlobals());

const failed: CiStatus = {
  state: 'failed', url: 'https://gh/runs/2',
  runs: [{ id: 2, attempt: 1, workflow: 'ci', state: 'failed', url: 'https://gh/runs/2', failedJobs: ['test (linux)'] }],
};
const passed: CiStatus = { state: 'passed', url: 'https://gh/runs/1', runs: [] };

const pr = (over: Partial<PullRequestStatus> = {}): PullRequestStatus => ({
  number: 12, url: 'https://gh/pull/12', title: 'Fix login', state: 'open', draft: false, baseBranch: 'main', headSha: 'abc',
  mergeable: 'mergeable', behind: false, autoMerge: true, inMergeQueue: false, queuePosition: null, ci: passed, queueCi: null,
  checkedAt: 1, error: null, ...over,
});

const attempt = (p: PullRequestStatus | null): AttemptSummary => ({
  id: 'a1', branch: 'pdt/ALP-1-fix-login', baseBranch: 'main', worktreeId: 'w1', state: 'active', createdAt: 1, endedAt: null,
  lastHead: null, workspaceId: null, ci: null, pr: p,
});

describe('pull request status', () => {
  it('sums a PR up by its most urgent state', () => {
    expect(prSummary(pr({ state: 'merged', mergeable: 'conflicting' })).label).toBe('Merged');
    expect(prSummary(pr({ mergeable: 'conflicting', ci: failed }))).toEqual({ label: 'Conflict', tone: 'failed' });
    expect(prSummary(pr({ ci: failed }))).toEqual({ label: 'CI failed', tone: 'failed' });
    expect(prSummary(pr({ queueCi: failed }))).toEqual({ label: 'Queue failed', tone: 'failed' });
    expect(prSummary(pr({ inMergeQueue: true, queueCi: failed })).label).toBe('In queue');
    expect(prSummary(pr({ ci: { ...passed, state: 'running' } })).label).toBe('CI running');
    expect(prSummary(pr({ draft: true })).label).toBe('Draft');
    expect(prSummary(pr()).label).toBe('Auto-merge');
    expect(prSummary(pr({ autoMerge: false })).label).toBe('CI passed');
    expect(prSummary(pr({ autoMerge: false, ci: null })).label).toBe('Open');
  });

  it('shows a merged attempt\'s PR as merged while the last look still saw it queued (PDT-59)', () => {
    expect(prSummary(pr({ inMergeQueue: true }), true)).toEqual({ label: 'Merged', tone: 'accent' });
    expect(prSummary(pr({ state: 'closed' }), true).label).toBe('Closed');
    render(<PrDetails pr={pr({ inMergeQueue: true, queuePosition: 1 })} attemptMerged />);
    expect(screen.getByText('Merged')).toBeInTheDocument();
    expect(screen.queryByText(/In merge queue/)).not.toBeInTheDocument();
  });

  it('shows the details: link, CI, conflict, queue, and what failed', () => {
    const { rerender } = render(<PrDetails pr={pr({ ci: failed, mergeable: 'conflicting' })} />);
    const box = screen.getByLabelText('Pull request');
    expect(within(box).getByRole('link', { name: /PR #12/ })).toHaveAttribute('href', 'https://gh/pull/12');
    expect(within(box).getByText('CI failed')).toBeInTheDocument();
    expect(within(box).getByText('Conflicts with main')).toBeInTheDocument();
    expect(within(box).getByText(/Failed: test \(linux\)/)).toBeInTheDocument();
    expect(within(box).getByRole('link', { name: /Run/ })).toHaveAttribute('href', 'https://gh/runs/2');
    rerender(<PrDetails pr={pr({ inMergeQueue: true, queuePosition: 2 })} />);
    expect(screen.getByText('In merge queue · #2')).toBeInTheDocument();
    expect(screen.queryByText('Auto-merge on')).toBeNull();
    rerender(<PrDetails pr={pr({ autoMerge: false, queueCi: failed })} />);
    expect(screen.getByText('Left the merge queue')).toBeInTheDocument();
    expect(screen.getByText(/Merge queue: failed/)).toBeInTheDocument();
  });

  it('says when the PR was last read, and why it is stale when reading it fails', () => {
    const now = Date.now();
    const { rerender } = render(<PrDetails pr={pr({ checkedAt: now - 3 * 60_000 })} />);
    expect(screen.getByText('Last checked 3m ago')).toBeInTheDocument();
    rerender(<PrDetails pr={pr({ checkedAt: now - 3 * 60_000, error: 'gh failed: HTTP 401' })} />);
    expect(screen.getByText('Not updated: gh failed: HTTP 401 (last checked 3m ago)')).toHaveClass('text-destructive');
    render(
      <ul>
        <BoardCard ticket={summary(1, { stage: 'in_progress', attempt: attempt(pr({ ci: { ...passed, state: 'running' }, error: 'gh failed' })) })}
          onOpen={() => {}} entries={() => []} />
      </ul>,
    );
    const badge = screen.getByText(/#12 · CI running/);
    expect(within(badge).getByLabelText('Not updated')).toBeInTheDocument();
    expect(badge.getAttribute('title')).toContain('Not updated: gh failed');
  });

  it('puts the PR on the board card and in the ticket Attempt section', async () => {
    render(
      <ul>
        <BoardCard ticket={summary(1, { stage: 'in_progress', attempt: attempt(pr({ ci: failed })) })} onOpen={() => {}} entries={() => []} />
      </ul>,
    );
    expect(screen.getByText(/#12 · CI failed/)).toHaveAttribute('title', expect.stringMatching(/^PR #12: Fix login — failed: test \(linux\)\nLast checked /));

    const t = ticket(1, { stage: 'in_progress', attempt: attempt(pr()), attempts: [attempt(pr())] });
    stubApi({
      'GET /api/tickets/t1': t,
      'GET /api/tickets/t1/events': { items: [], nextCursor: null },
      'POST /api/tickets/t1/check-merge': t,
      [`GET /api/projects/${project.id}/labels`]: [],
      'GET /api/projects': [project],
      [`GET /api/projects/${project.id}/sessions`]: [],
    });
    renderWithQuery(<TicketView ticketId="t1" />);
    expect(await screen.findByRole('link', { name: /PR #12/ })).toBeInTheDocument();
    expect(screen.getByText('Auto-merge on')).toBeInTheDocument();
  });

  it('names pr events on the timeline', () => {
    const e = (data: unknown) => event('e', { kind: 'pr', actor: 'system', data });
    expect(eventTitle(e({ notice: 'ci_failed', pr: 12, jobs: ['test (linux)'], told: true })))
      .toBe('CI failed (PR #12): test (linux) · told the agent');
    expect(eventTitle(e({ notice: 'merged', pr: null, jobs: [], told: false }))).toBe('Merged · no agent running to tell');
    expect(eventTitle(e({ notice: 'queue_left', pr: 3, jobs: [], told: true }))).toBe('Left the merge queue (PR #3) · told the agent');
  });
});
